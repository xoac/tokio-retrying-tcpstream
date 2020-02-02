# tokio-retrying-tcpstream

This crate wraps [TcpStream] and [ConnectFuture] into [RetryingTcpStream].

When you work with connection that are expected to sometimes broke you may want add auto reconnect after
error is detected. [RetryingTcpStream] makes pollable after returning error.
It mean any time, any method that name start with `poll` return Error - the inner state will reset.

When you think about [RetryingTcpStream] you should think about mix [ConnectFuture] and [TcpStream].
When you call `poll_read()` or `poll_write()` on [RetryingTcpStream]
1. If it is in [ConnectFuture] state -> will go to [TcpStream] state and go to 2.
2. If it is in [TcpStream] state -> will call requested method retrurning result:
  - `Ok(_)` -> Normal poll result
  - `Err(_)` -> Internal state is reset to [ConnectFuture] state. Next poll*() method will try connect.

[RetryingTcpStream] is design to work with [futures-retry]. It's up to you with error are temporary and can be repair by reconnecting.


[RetryingTcpStream]: RetryingTcpStream
[futures-retry]: https://docs.rs/futures-retry/0.3
[ConnectFuture]: tokio::net::tcp::ConnectFuture
[TcpStream]: tokio::net::TcpStream

License: MIT
