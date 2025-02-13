use std::io;
use std::net::SocketAddr;
use std::sync::atomic::Ordering;

use super::super::{add_socket, co_io_result, IoData};
#[cfg(feature = "io_cancel")]
use crate::coroutine_impl::co_cancel_data;
use crate::coroutine_impl::{is_coroutine, CoroutineImpl, EventSource};
use crate::io::AsIoData;
use crate::net::{TcpListener, TcpStream};
use crate::yield_now::yield_with_io;

pub struct TcpListenerAccept<'a> {
    io_data: &'a IoData,
    socket: &'a std::net::TcpListener,
    pub(crate) is_coroutine: bool,
}

impl<'a> TcpListenerAccept<'a> {
    pub fn new(socket: &'a TcpListener) -> io::Result<Self> {
        Ok(TcpListenerAccept {
            io_data: socket.as_io_data(),
            socket: socket.inner(),
            is_coroutine: is_coroutine(),
        })
    }

    pub fn done(&mut self) -> io::Result<(TcpStream, SocketAddr)> {
        loop {
            co_io_result(self.is_coroutine)?;

            // clear the io_flag
            self.io_data.io_flag.store(0, Ordering::Relaxed);

            match self.socket.accept() {
                Ok((s, a)) => {
                    s.set_nonblocking(true)?;
                    return add_socket(&s).map(|io| (TcpStream::from_stream(s, io), a));
                }
                Err(e) => {
                    // raw_os_error is faster than kind
                    let raw_err = e.raw_os_error();
                    if raw_err == Some(libc::EAGAIN) || raw_err == Some(libc::EWOULDBLOCK) {
                        // do nothing here
                    } else {
                        return Err(e);
                    }
                }
            }

            if self.io_data.io_flag.load(Ordering::Relaxed) != 0 {
                continue;
            }

            // the result is still WouldBlock, need to try again
            yield_with_io(self, self.is_coroutine);
        }
    }
}

impl EventSource for TcpListenerAccept<'_> {
    fn subscribe(&mut self, co: CoroutineImpl) {
        #[cfg(feature = "io_cancel")]
        let cancel = co_cancel_data(&co);
        let io_data = self.io_data;
        // if there is no timer we don't need to call add_io_timer
        io_data.co.store(co);

        // there is event happened
        if io_data.io_flag.load(Ordering::Acquire) != 0 {
            #[allow(clippy::needless_return)]
            return io_data.fast_schedule();
        }

        #[cfg(feature = "io_cancel")]
        {
            // register the cancel io data
            cancel.set_io((*io_data).clone());
            // re-check the cancel status
            if cancel.is_canceled() {
                unsafe { cancel.cancel() };
            }
        }
    }
}
