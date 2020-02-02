//! This crate wraps [TcpStream] and [ConnectFuture] into [RetryingTcpStream].
//!
//! When you work with connection that are expected to sometimes broke you may want add auto reconnect after
//! error is detected. [RetryingTcpStream] makes pollable after returning error.
//! It mean any time, any method that name start with `poll` return Error - the inner state will reset.
//!
//! When you think about [RetryingTcpStream] you should think about mix [ConnectFuture] and [TcpStream].
//! When you call `poll_read()` or `poll_write()` on [RetryingTcpStream]
//! 1. If it is in [ConnectFuture] state -> will go to [TcpStream] state and go to 2.
//! 2. If it is in [TcpStream] state -> will call requested method retrurning result:
//!   - `Ok(_)` -> Normal poll result
//!   - `Err(_)` -> Internal state is reset to [ConnectFuture] state. Next poll*() method will try connect.
//!
//! [RetryingTcpStream] is design to work with [futures-retry]. It's up to you with error are temporary and can be repair by reconnecting.
//!
//!
//! [RetryingTcpStream]: RetryingTcpStream
//! [futures-retry]: https://docs.rs/futures-retry/0.3
//! [ConnectFuture]: tokio::net::tcp::ConnectFuture
//! [TcpStream]: tokio::net::TcpStream

use std::convert::TryFrom;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::time::Duration;

use futures::try_ready;
use log::debug;
use mio;
use tokio::io::{AsyncRead, AsyncWrite, Error};
use tokio::prelude::{Async, Future, Poll};

/// Holding settings [TcpStreamSettings]
#[derive(Hash, PartialEq, Eq, Clone)]
pub struct TcpStreamSettings {
    pub nodelay: bool,
    pub keepalive: Option<Duration>,
}

// Handle connection state
enum ConnectionState {
    ConnectFuture(tokio::net::tcp::ConnectFuture),
    TcpStream(tokio::net::TcpStream),
}

/// Like TcpStream but pollable after Error.
pub struct RetryingTcpStream {
    addr: std::net::SocketAddr,
    settings: TcpStreamSettings,
    state: ConnectionState,
}

impl TryFrom<tokio::net::TcpStream> for RetryingTcpStream {
    type Error = Error;
    fn try_from(tcp_stream: tokio::net::TcpStream) -> Result<Self, Self::Error> {
        let settings = TcpStreamSettings {
            nodelay: tcp_stream.nodelay()?,
            keepalive: tcp_stream.keepalive()?,
        };

        Ok(RetryingTcpStream {
            addr: tcp_stream.peer_addr()?,
            state: ConnectionState::TcpStream(tcp_stream),
            settings,
        })
    }
}

/// Implement creators
impl RetryingTcpStream {
    pub fn connect_with_settings(addr: &std::net::SocketAddr, settings: TcpStreamSettings) -> Self {
        Self {
            addr: addr.clone(),
            state: ConnectionState::ConnectFuture(tokio::net::TcpStream::connect(addr)),
            settings,
        }
    }

    pub fn from_std(
        stream: std::net::TcpStream,
        handle: &tokio::reactor::Handle,
    ) -> Result<Self, Error> {
        let tokio_tcp_stream = tokio::net::TcpStream::from_std(stream, handle)?;
        Self::try_from(tokio_tcp_stream)
    }
}

/// Reimplement of methods from [TcpStream]
/// The main differences are:
/// - `tokio::io::ErrorKind::NotConnected` is returned when inner state is in [ConnectFuture] and
/// there is no other way to get response.
/// - poll methods will try first go from [ConnectFuture] -> [TcpStream]
///
/// For more documentation go to [TcpStream] doc.
///
/// [TcpStream]:tokio::net::TcpStream
/// [ConnectFuture]:tokio::net::tcp::ConnectFuture
impl RetryingTcpStream {
    pub fn poll_read_ready(&mut self, mask: mio::Ready) -> Result<Async<mio::Ready>, Error> {
        let ts = try_ready!(self.poll_into_tcp_stream());
        let res = ts.poll_read_ready(mask);
        self.call_reset_if_io_is_closed2(res)
    }

    pub fn poll_write_ready(&mut self) -> Result<Async<mio::Ready>, Error> {
        let ts = try_ready!(self.poll_into_tcp_stream());
        let res = ts.poll_write_ready();
        self.call_reset_if_io_is_closed2(res)
    }

    pub fn poll_peek(&mut self, buf: &mut [u8]) -> Result<Async<usize>, Error> {
        let ts = try_ready!(self.poll_into_tcp_stream());
        let res = ts.poll_peek(buf);
        self.call_reset_if_io_is_closed2(res)
    }

    pub fn local_addr(&self) -> Result<std::net::SocketAddr, Error> {
        self.ref_tcp_stream()?.local_addr()
    }

    pub fn peer_addr(&self) -> Result<std::net::SocketAddr, Error> {
        match &self.state {
            ConnectionState::ConnectFuture(_) => Ok(self.addr),
            ConnectionState::TcpStream(ts) => {
                let r = ts.peer_addr()?;
                debug_assert_eq!(r, self.addr);
                Ok(r)
            }
        }
    }

    pub fn nodelay(&self) -> Result<bool, Error> {
        match self.ref_tcp_stream() {
            Ok(ts) => {
                let r = ts.nodelay()?;
                debug_assert!(r, self.settings.nodelay);
                Ok(r)
            }
            Err(_) => Ok(self.settings.nodelay),
        }
    }

    pub fn set_nodelay(&mut self, nodelay: bool) -> Result<(), Error> {
        match &self.state {
            ConnectionState::ConnectFuture(_) => {
                self.settings.nodelay = nodelay;
                Ok(())
            }
            ConnectionState::TcpStream(ts) => match ts.set_nodelay(nodelay) {
                Result::Ok(_) => {
                    self.settings.nodelay = nodelay;
                    Ok(())
                }
                Result::Err(err) => Err(err),
            },
        }
    }

    pub fn shutdown(&self, how: Shutdown) -> Result<(), Error> {
        self.ref_tcp_stream()?.shutdown(how)
    }

    pub fn keepalive(&self) -> Result<Option<Duration>, Error> {
        match self.ref_tcp_stream() {
            Ok(ts) => {
                let r = ts.keepalive()?;
                debug_assert_eq!(r, self.settings.keepalive);
                Ok(r)
            }
            Err(_) => Ok(self.settings.keepalive),
        }
    }

    pub fn set_keepalive(&self, keepalive: Option<Duration>) -> Result<(), Error> {
        self.ref_tcp_stream()?.set_keepalive(keepalive)
    }
}

/// Implement additional methods
impl RetryingTcpStream {
    pub fn set_tcp_settings(&mut self, tcp_settings: TcpStreamSettings) -> Result<(), Error> {
        self.set_nodelay(tcp_settings.nodelay)?;
        self.set_keepalive(tcp_settings.keepalive)?;

        self.settings = tcp_settings;
        Ok(())
    }

    /// return true if RetryingTcpStream represent [TcpStream](tokio::net::TcpStream) at this
    /// moment.
    ///
    /// This can change after calling any `poll*()` method when that function returned error.
    pub fn is_in_tcp_state(&self) -> bool {
        match self.state {
            ConnectionState::TcpStream(_) => true,
            ConnectionState::ConnectFuture(_) => false,
        }
    }

    fn ref_tcp_stream(&self) -> Result<&tokio::net::TcpStream, Error> {
        match &self.state {
            ConnectionState::ConnectFuture(_) => {
                Err(Error::from(tokio::io::ErrorKind::NotConnected))
            }
            ConnectionState::TcpStream(ts) => Ok(ts),
        }
    }

    // Return NotReady until ConnectionState is diffrent than TcpStream
    fn poll_into_tcp_stream(&mut self) -> Poll<&mut tokio::net::TcpStream, Error> {
        match &mut self.state {
            ConnectionState::ConnectFuture(cf) => {
                let tcp_s = match cf.poll() {
                    Ok(Async::Ready(tcp_s)) => tcp_s,
                    Ok(Async::NotReady) => return Ok(Async::NotReady),
                    Err(err) => {
                        self.reset();
                        return Err(err);
                    }
                };
                self.state = ConnectionState::TcpStream(tcp_s);
                self.set_tcp_settings(self.settings.clone())?;
                debug!("RetryingTcpStream => change state ConnectFuture -> TcpStream")
            }
            ConnectionState::TcpStream(_) => (),
        };

        match self.state {
            ConnectionState::ConnectFuture(_) => unreachable!(),
            ConnectionState::TcpStream(ref mut ts) => Ok(Async::Ready(ts)),
        }
    }

    fn reset(&mut self) {
        debug!("RetryinTcpStream => reset was called!");
        self.state = ConnectionState::ConnectFuture(tokio::net::TcpStream::connect(&self.addr))
    }

    fn call_reset_if_io_is_closed2<T>(&mut self, res: Result<T, Error>) -> Result<T, Error> {
        use tokio::io::ErrorKind;
        match res {
            Ok(ok) => Ok(ok),
            Err(err) => {
                match err.kind() {
                    ErrorKind::WouldBlock => (),
                    _ => self.reset(),
                };
                Err(err)
            }
        }
    }
}

impl Read for RetryingTcpStream {
    /// # Note
    /// This is Async version of Read. It will panic outside of tash
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let ts = self.poll_into_tcp_stream()?;
        let r = match ts {
            Async::Ready(ts) => ts.read(buf),
            Async::NotReady => Err(std::io::ErrorKind::WouldBlock.into()),
        };

        self.call_reset_if_io_is_closed2(r)
    }
}

impl Write for RetryingTcpStream {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Error> {
        let ts = self.poll_into_tcp_stream()?;
        let r = match ts {
            Async::Ready(ts) => ts.write(buf),
            Async::NotReady => Err(std::io::ErrorKind::WouldBlock.into()),
        };

        self.call_reset_if_io_is_closed2(r)
    }

    fn flush(&mut self) -> Result<(), Error> {
        let ts = self.poll_into_tcp_stream()?;
        let r = match ts {
            Async::Ready(ts) => ts.flush(),
            Async::NotReady => Err(std::io::ErrorKind::WouldBlock.into()),
        };

        self.call_reset_if_io_is_closed2(r)
    }
}

// Logic is implemented inside Read and Write trait becouse we can't overwrite AsyncRead and
// AsyncWrite for Box<RetryingTcpStream>
// source: https://docs.rs/tokio-io/0.1.12/src/tokio_io/async_write.rs.html#149
impl AsyncRead for RetryingTcpStream {}

impl AsyncWrite for RetryingTcpStream {
    fn shutdown(&mut self) -> Poll<(), Error> {
        match &mut self.state {
            ConnectionState::ConnectFuture(_cf) => {
                // there is a chance when we call poll conection will resolve to TcpStream
                // we probably need add a Shutdowned state.
                unimplemented!();
            }
            ConnectionState::TcpStream(ts) => ts.shutdown(),
        }
    }
}
