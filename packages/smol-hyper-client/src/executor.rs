use std::error::Error as StdError;

use hyper::client::conn::http1::Connection;
use smol_hyper::rt::FuturesIo;

use super::SmolStream;

type ConnectionFuture<B> = Connection<FuturesIo<SmolStream>, B>;
type ConnectionOutput<B> = <ConnectionFuture<B> as Future>::Output;
type ConnectionOutputFn<B> = fn(ConnectionOutput<B>) -> ();
type SpawnFuture<B> = futures_util::future::Map<ConnectionFuture<B>, ConnectionOutputFn<B>>;

/// Export the bounds we need on an hyper::rt::Executor in terms of the actual futures we want to run.
/// It has a bound on a future containing a private type, which is effectively the same as having a
/// bound on "you must implement this trait for _all_ futures".
#[allow(private_bounds)]
pub trait Executor<B>: hyper::rt::Executor<SpawnFuture<B>>
where
    B: hyper::body::Body + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn StdError + Send + Sync>>,
{
}

impl<B, T> Executor<B> for T
where
    B: hyper::body::Body + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn StdError + Send + Sync>>,
    T: hyper::rt::Executor<SpawnFuture<B>>,
{
}
