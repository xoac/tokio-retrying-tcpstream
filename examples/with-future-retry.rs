use futures::{
    sink::SinkExt,
    stream::{StreamExt, TryStreamExt},
};
use futures_retry::{RetryPolicy, SinkRetryExt, StreamRetryExt};
use std::convert::TryFrom;
use std::time::Duration;
use tokio::{self, net::TcpStream};
use tokio_retrying_tcpstream::RetryingTcpStream;
use tokio_util::codec::{Framed, LinesCodec, LinesCodecError};

fn retry_in_5s(_e: LinesCodecError) -> RetryPolicy<LinesCodecError> {
    RetryPolicy::WaitRetry(Duration::from_secs(5))
}

#[tokio::main]
async fn main() {
    let conn = TcpStream::connect("127.0.0.1:8080").await.unwrap();

    let rts = RetryingTcpStream::try_from(conn).unwrap();

    let framed = Framed::new(rts, LinesCodec::new());

    let (write, read) = framed.split();

    let retry_read = read.retry(retry_in_5s).map_err(|_| unreachable!());
    let retry_write = write.retry(retry_in_5s).sink_map_err(|_| unreachable!());

    retry_read.forward(retry_write).await.unwrap();
}
