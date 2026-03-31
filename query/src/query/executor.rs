use std::{
    collections::VecDeque,
    sync::atomic::{AtomicBool, Ordering},
};

use async_task::Runnable;
use either::Either;
use flume::{Receiver, Sender};
use futures_lite::future;

use crate::query::key::QueryKey;

/// The main struct that runs the futures. Uses a thread-per-core architecture to run things as
/// efficiently as possible.
///
/// How it works is, each thread has its own async executor that's running at least 1 future: a
/// future that continuously tries to take another bit of work off the single task queue. It SHOULD
/// do this only when all other futures in the executor are Poll::Pending, and there is no other
/// future that is ready to wake up (TODO: I don't think we guarantee this, nor is the queue
/// guaranteed to be fair, but maybe this is fine??).
///
/// Once a thread has pulled a query off the queue, it executes it to completion, pinned to the
/// thread, and then sends the result back over a oneshot channel.
pub struct Executor {
    /// The sending end that lets us spawn new queries onto the threadpool.
    pub(crate) sender: Sender<QueryKey>,
    /// A global that lets us stop the threadpool.
    running: &'static AtomicBool,
}

impl Executor {
    pub fn start() -> Executor {
        let n = num_cpus::get();
        let (send_key, recv_key) = flume::unbounded();
        let running = &*Box::leak(Box::new(AtomicBool::new(true)));

        for _ in 0..n {
            let recv_key = recv_key.clone();
            std::thread::spawn(move || {
                let (send_runnable, recv_runnable) = flume::unbounded::<Runnable>();
                future::block_on(async {
                    loop {
                        // TODO: might also want to select on this one too? When waiting for either
                        // the runnable or the key. Somehow.
                        if running.load(Ordering::Relaxed) {
                            break;
                        }
                        // Prefer to pull from the queue of current futures to execute over the
                        // queue of keys to execute next.
                        match future::block_on(future::or(
                            async {
                                Either::Left(
                                    recv_runnable
                                        .recv_async()
                                        .await
                                        .expect("runnable receive error"),
                                )
                            },
                            async {
                                Either::Right(
                                    recv_key.recv_async().await.expect("key receive error"),
                                )
                            },
                        )) {
                            Either::Left(runnable) => {
                                runnable.run();
                            }
                            Either::Right(key) => {
                                todo!("spawn task for key");
                            }
                        }
                    }
                });
            });
        }

        Self {
            sender: send_key,
            running,
        }
    }

    pub fn stop(self) {
        todo!()
    }
}
