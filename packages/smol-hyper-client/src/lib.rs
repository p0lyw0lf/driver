//! TODO: connection pooling

use std::error::Error as StdError;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use async_native_tls::TlsStream;
use async_net::TcpStream;
use futures_lite::{AsyncRead, AsyncWrite, io};
use futures_util::future::FutureExt;
use hyper::body::Body;
use thiserror::Error;

mod executor;
pub use executor::Executor;

mod uri;
pub use uri::Uri;

/// The main struct used to fetch URLs. Contains a hyper::rt::Executor that is used to spawn
/// connection tasks.
#[derive(Debug)]
pub struct Client<B> {
    body_type: PhantomData<B>,
}

pub static USER_AGENT: &str = concat!(
    "hyper (",
    env!("CARGO_PKG_NAME"),
    " ",
    env!("CARGO_PKG_VERSION"),
    ") (+https://github.com/p0lyw0lf/driver)",
);

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("cannot parse host: {uri}")]
    InvalidHost { uri: String },
    #[error("unsupported scheme: {scheme}")]
    UnsupportedScheme { scheme: String },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("tls error: {0}")]
    Tls(#[from] async_native_tls::Error),
    #[error("hyper error: {0}")]
    Hyper(#[from] hyper::Error),
}

/// We need to specifiy _which_ future we're going to be spawning on the executor, so let's do that
/// with some gnarly typing. All that a consumer of this library needs to do is to provide a type
/// that implements `hyper::rt::Executor` for a sufficient variety of futures.
#[allow(private_bounds)]
impl<B> Client<B>
where
    B: Body + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn StdError + Send + Sync>>,
{
    pub fn new() -> Self {
        Self {
            body_type: PhantomData,
        }
    }

    /// Mostly taken from <https://github.com/smol-rs/smol/blob/4af083b2078f2e4d6b9810abb0e6ed4186729ef9/examples/hyper-client.rs>
    pub async fn request<E>(
        &self,
        executor: &E,
        req: hyper::Request<B>,
    ) -> Result<hyper::Response<hyper::body::Incoming>, ClientError>
    where
        E: Executor<B>,
    {
        let io = {
            let uri = req.uri();
            let host = uri.host().ok_or_else(|| ClientError::InvalidHost {
                uri: uri.to_string(),
            })?;
            match req.uri().scheme_str() {
                Some("http") => {
                    let stream = {
                        let port = req.uri().port_u16().unwrap_or(80);
                        TcpStream::connect((host, port)).await?
                    };
                    SmolStream::Plain(stream)
                }
                Some("https") => {
                    let stream = {
                        let port = req.uri().port_u16().unwrap_or(443);
                        TcpStream::connect((host, port)).await?
                    };
                    let stream = async_native_tls::connect(host, stream).await?;
                    SmolStream::Tls(stream)
                }
                otherwise => {
                    return Err(ClientError::UnsupportedScheme {
                        scheme: otherwise.unwrap_or("None").into(),
                    });
                }
            }
        };

        // Spawn the HTTP/1 connection.
        let (mut sender, conn) =
            hyper::client::conn::http1::handshake(smol_hyper::rt::FuturesIo::new(io)).await?;
        executor.execute(FutureExt::map(conn, |output| {
            if let Err(e) = output {
                eprintln!("connection failed: {e}");
            }
        }));

        let result = sender.send_request(req).await?;
        Ok(result)
    }
}

impl<B> Default for Client<B>
where
    B: Body + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn StdError + Send + Sync>>,
{
    fn default() -> Self {
        Self::new()
    }
}

/// A TCP or TCP+TLS connection.
enum SmolStream {
    /// A plain TCP connection.
    Plain(TcpStream),

    /// A TCP connection secured by TLS.
    Tls(TlsStream<TcpStream>),
}

impl AsyncRead for SmolStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        match &mut *self {
            SmolStream::Plain(stream) => Pin::new(stream).poll_read(cx, buf),
            SmolStream::Tls(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for SmolStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &mut *self {
            SmolStream::Plain(stream) => Pin::new(stream).poll_write(cx, buf),
            SmolStream::Tls(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            SmolStream::Plain(stream) => Pin::new(stream).poll_close(cx),
            SmolStream::Tls(stream) => Pin::new(stream).poll_close(cx),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            SmolStream::Plain(stream) => Pin::new(stream).poll_flush(cx),
            SmolStream::Tls(stream) => Pin::new(stream).poll_flush(cx),
        }
    }
}
