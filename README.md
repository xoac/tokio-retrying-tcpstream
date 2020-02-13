[![crates.io](https://img.shields.io/crates/v/tokio-retrying-tcpstream.svg)](https://crates.io/crates/tokio-retrying-tcpstream)
[![Documentation](https://docs.rs/tokio-retrying-tcpstream/badge.svg)](https://docs.rs/tokio-retrying-tcpstream/)
![CI master](https://github.com/xoac/tokio-retrying-tcpstream/workflows/Continuous%20integration/badge.svg?branch=master)
![Maintenance](https://img.shields.io/badge/maintenance-activly--developed-brightgreen.svg)

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

## License

Licensed under either of

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.

This project try follow rules:
* [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
* [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

_This README was generated with [cargo-readme](https://github.com/livioribeiro/cargo-readme) from [template](https://github.com/xoac/crates-io-lib-template)_
