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
use std::future::Future;
use std::io;
use std::net::Shutdown;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::ready;
use log::debug;
use tokio::io::{AsyncRead, AsyncWrite};

/// Holding settings [TcpStreamSettings]
#[derive(Hash, PartialEq, Eq, Clone)]
pub struct TcpStreamSettings {
    pub nodelay: bool,
    pub keepalive: Option<Duration>,
}

// Handle connection state
enum ConnectionState {
    ConnectFuture(Pin<Box<dyn Future<Output = io::Result<tokio::net::TcpStream>>>>),
    TcpStream(tokio::net::TcpStream),
}

/// Like TcpStream but pollable after Error.
pub struct RetryingTcpStream {
    addr: std::net::SocketAddr,
    settings: TcpStreamSettings,
    state: ConnectionState,
}

impl TryFrom<tokio::net::TcpStream> for RetryingTcpStream {
    type Error = io::Error;
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
        let state =
            ConnectionState::ConnectFuture(Box::pin(tokio::net::TcpStream::connect(addr.clone())));
        Self {
            addr: addr.clone(),
            state,
            settings,
        }
    }

    pub fn from_std(stream: std::net::TcpStream) -> io::Result<Self> {
        let tokio_tcp_stream = tokio::net::TcpStream::from_std(stream)?;
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
    /*
      pub fn poll_peek(&mut self, cx: &mut Context, buf: &mut [u8]) -> Poll<io::Result<usize>> {
          let ts = ready!(self.poll_into_tcp_stream(cx))?;
          let res = ready!(ts.poll_peek(cx, buf));
          Poll::Ready(self.call_reset_if_io_is_closed2(res))
      }
    */

    pub fn local_addr(&self) -> io::Result<std::net::SocketAddr> {
        self.ref_tcp_stream()?.local_addr()
    }

    pub fn peer_addr(&self) -> io::Result<std::net::SocketAddr> {
        match &self.state {
            ConnectionState::ConnectFuture(_) => Ok(self.addr),
            ConnectionState::TcpStream(ts) => {
                let r = ts.peer_addr()?;
                debug_assert_eq!(r, self.addr);
                Ok(r)
            }
        }
    }

    pub fn nodelay(&self) -> io::Result<bool> {
        match self.ref_tcp_stream() {
            Ok(ts) => {
                let r = ts.nodelay()?;
                debug_assert!(r, self.settings.nodelay);
                Ok(r)
            }
            Err(_) => Ok(self.settings.nodelay),
        }
    }

    pub fn set_nodelay(&mut self, nodelay: bool) -> io::Result<()> {
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

    pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        self.ref_tcp_stream()?.shutdown(how)
    }

    pub fn keepalive(&self) -> io::Result<Option<Duration>> {
        match self.ref_tcp_stream() {
            Ok(ts) => {
                let r = ts.keepalive()?;
                debug_assert_eq!(r, self.settings.keepalive);
                Ok(r)
            }
            Err(_) => Ok(self.settings.keepalive),
        }
    }

    pub fn set_keepalive(&self, keepalive: Option<Duration>) -> io::Result<()> {
        self.ref_tcp_stream()?.set_keepalive(keepalive)
    }
}

/// Implement additional methods
impl RetryingTcpStream {
    pub fn set_tcp_settings(&mut self, tcp_settings: TcpStreamSettings) -> io::Result<()> {
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

    fn ref_tcp_stream(&self) -> io::Result<&tokio::net::TcpStream> {
        match &self.state {
            ConnectionState::ConnectFuture(_) => Err(io::Error::from(io::ErrorKind::NotConnected)),
            ConnectionState::TcpStream(ts) => Ok(ts),
        }
    }

    // Return NotReady until ConnectionState is diffrent than TcpStream
    pub fn poll_into_tcp_stream(
        &mut self,
        cx: &mut Context,
    ) -> Poll<io::Result<&mut tokio::net::TcpStream>> {
        let self2 = self;

        match &mut self2.state {
            ConnectionState::ConnectFuture(cf) => {
                let tcp_s = ready!(cf.as_mut().poll(cx))?;
                self2.state = ConnectionState::TcpStream(tcp_s);
                self2.set_tcp_settings(self2.settings.clone())?;
                debug!("RetryingTcpStream => change state ConnectFuture -> TcpStream")
            }
            ConnectionState::TcpStream(_) => (),
        };

        match self2.state {
            ConnectionState::ConnectFuture(_) => unreachable!(),
            ConnectionState::TcpStream(ref mut ts) => Poll::Ready(Ok(ts)),
        }
    }

    fn reset(&mut self) {
        debug!("RetryinTcpStream => reset was called!");
        self.state = ConnectionState::ConnectFuture(Box::pin(tokio::net::TcpStream::connect(
            self.addr.clone(),
        )))
    }

    fn call_reset_if_io_is_closed2<T>(&mut self, res: Result<T, io::Error>) -> io::Result<T> {
        use std::io::ErrorKind;
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

// Logic is implemented inside Read and Write trait becouse we can't overwrite AsyncRead and
// AsyncWrite for Box<RetryingTcpStream>
// source: https://docs.rs/tokio-io/0.1.12/src/tokio_io/async_write.rs.html#149
impl AsyncRead for RetryingTcpStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let self_mut = self.get_mut();
        let tcp_s = ready!(self_mut.poll_into_tcp_stream(cx))?;
        let r = ready!(Pin::new(tcp_s).poll_read(cx, buf));
        Poll::Ready(self_mut.call_reset_if_io_is_closed2(r))
    }
}

impl AsyncWrite for RetryingTcpStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let self_mut = self.get_mut();
        let tcp_s = ready!(self_mut.poll_into_tcp_stream(cx))?;
        let r = ready!(Pin::new(tcp_s).poll_write(cx, buf));
        Poll::Ready(self_mut.call_reset_if_io_is_closed2(r))
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let self_mut = self.get_mut();
        let tcp_s = ready!(self_mut.poll_into_tcp_stream(cx))?;
        let r = ready!(Pin::new(tcp_s).poll_flush(cx));
        Poll::Ready(self_mut.call_reset_if_io_is_closed2(r))
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let self_mut = self.get_mut();
        let tcp_s = ready!(self_mut.poll_into_tcp_stream(cx))?;
        let r = ready!(Pin::new(tcp_s).poll_shutdown(cx));
        Poll::Ready(self_mut.call_reset_if_io_is_closed2(r))
    }
}
