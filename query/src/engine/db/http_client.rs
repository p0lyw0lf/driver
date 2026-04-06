use std::fmt::Display;
use std::ops::Deref;
use std::pin::Pin;
use std::task::{Context, Poll};

use async_native_tls::TlsStream;
use async_net::TcpStream;
use futures_lite::{AsyncRead, AsyncWrite, io};
use hyper::rt::Executor;
use serde::{Deserialize, Serialize, de::Error};

use crate::engine::executor::CurrentThreadExecutor;

/// TODO: connection pooling
#[derive(Debug)]
pub struct Client;
pub fn default_client() -> Client {
    Client
}

pub static USER_AGENT: &str = concat!(
    "hyper (",
    env!("CARGO_PKG_NAME"),
    " ",
    env!("CARGO_PKG_VERSION"),
    ") (+https://github.com/p0lyw0lf/driver)",
);

impl Client {
    /// Mostly taken from https://github.com/smol-rs/smol/blob/4af083b2078f2e4d6b9810abb0e6ed4186729ef9/examples/hyper-client.rs
    pub async fn request(
        &self,
        req: hyper::Request<http_body_util::Empty<hyper::body::Bytes>>,
    ) -> crate::Result<hyper::Response<hyper::body::Incoming>> {
        let io = {
            let host = req
                .uri()
                .host()
                .ok_or_else(|| crate::Error::new("cannot parse host"))?;
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
                _otherwise => return Err(crate::Error::new("unsupported scheme")),
            }
        };

        // Spawn the HTTP/1 connection.
        let (mut sender, conn) =
            hyper::client::conn::http1::handshake(smol_hyper::rt::FuturesIo::new(io)).await?;
        CurrentThreadExecutor.execute(async move {
            if let Err(e) = conn.await {
                eprintln!("connection failed: {e}");
            }
        });

        let result = sender.send_request(req).await?;
        Ok(result)
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

/// Newtype in order to get Serialize/Deserialize, and PartialOrd/Ord
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct Uri(pub hyper::Uri);

impl From<Uri> for hyper::Uri {
    fn from(value: Uri) -> Self {
        value.0
    }
}

impl Deref for Uri {
    type Target = hyper::Uri;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Display for Uri {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl PartialOrd for Uri {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Uri {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let a = self.0.to_string();
        let b = other.0.to_string();
        a.cmp(&b)
    }
}

impl Serialize for Uri {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for Uri {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = <&'_ str>::deserialize(deserializer)?;
        let uri = hyper::Uri::try_from(s).map_err(|err| D::Error::custom(err))?;
        Ok(Uri(uri))
    }
}
