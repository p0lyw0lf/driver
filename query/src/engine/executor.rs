use std::cell::OnceCell;
use std::{sync::Arc, thread::JoinHandle};

use async_task::Runnable;
use futures_concurrency::stream::IntoStream;
use futures_lite::future;
use futures_lite::stream::{self, StreamExt};

use crate::engine::{any_output::AnyOutput, context::QueryContext, db::Database, key::QueryKey};
use crate::options::Options;

/// The main struct that runs the futures. Uses a thread-per-core architecture to run things as
/// efficiently as possible.
///
/// How it works is, each thread has its own async executor that's running at least 1 future: a
/// future that continuously tries to take another bit of work off the single task queue. It SHOULD
/// do this only when all other futures in the executor are Poll::Pending, and there is no other
/// future that is ready to wake up (TODO: I don't think we're guarnateed to be fair with how we're
/// pulling these off, should probably think of a slightly smarter knapsack problem approximation
/// at some point).
///
/// Once a thread has pulled a query off the queue, it executes it to completion, pinned to the
/// thread, and then sends the result back over a oneshot channel.
///
/// This is also the top-level
#[derive(Debug)]
pub struct Executor {
    /// Options to customize the runtime behavior of the executor
    pub(crate) options: Options,
    /// Created on start, and saved on stop
    pub(crate) db: Database,
    /// All the threads in the threadpool.
    threads: Vec<JoinHandle<()>>,
    /// The sending end of a channel that lets us spawn new queries onto the threadpool.
    send_work: flume::Sender<UnitOfWork>,
    /// A broadcast channel to let us stop the threadpool
    send_stop: async_broadcast::Sender<()>,
}

/// A single "unit of work" (lmao at the name) that the executor keeps track of. Multiple
/// corredponding to a single key/ctx can be in-flight at the same time, but only the first one
/// will actually run any "real" computation (thanks to a locking mechanism).
struct UnitOfWork {
    key: QueryKey,
    ctx: QueryContext,
    send: oneshot::Sender<AnyOutput>,
}

impl Executor {
    /// MUST NOT be run in an async context.
    pub fn start(options: Options) -> Executor {
        let db = Database::restore(&options).unwrap_or_else(|err| {
            eprintln!("error restoring database: {err}");
            Database::new(&options)
        });
        // Bust cache immediately
        db.revision
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let (send_work, recv_work) = flume::unbounded();
        let (send_stop, recv_stop) = async_broadcast::broadcast(1);

        let n = 2; // num_cpus::get();
        let threads = (0..n)
            .map(|_| {
                let recv_work = recv_work.clone();
                let recv_stop = recv_stop.clone();
                std::thread::spawn(move || main_loop(recv_work, recv_stop))
            })
            .collect();

        Self {
            options,
            db,
            threads,
            send_work,
            send_stop,
        }
    }

    pub(crate) async fn query(
        self: Arc<Self>,
        key: QueryKey,
        parent: Option<QueryKey>,
    ) -> AnyOutput {
        let (send, recv) = oneshot::channel();
        let ctx = QueryContext {
            parent,
            executor: self.clone(),
        };
        let query = UnitOfWork { key, ctx, send };
        self.send_work.send(query).expect("query send error");
        recv.await.expect("output receive error")
    }

    /// MUST NOT be run in an async context.
    pub fn stop(self) -> crate::Result<()> {
        // Tell all threads to stop running
        let _ = self
            .send_stop
            .broadcast_blocking(())
            .expect("stop send error");

        // Wait for all threads to finish running
        for thread in self.threads.into_iter() {
            thread.join().expect("thread join error");
        }

        // Only then should we save the database
        Database::save(self.db, &self.options)
    }

    pub fn display_dep_graph(&self) -> impl std::fmt::Display + '_ {
        self.db.display_dep_graph()
    }
}

/// The type of event that can be received on each thread's main loop
enum Event {
    OldWork(Runnable),
    NewWork(UnitOfWork),
    Stop,
}

thread_local! {
    /// Thread-local value that lets us spawn new futures onto the current thread's executor.
    static SEND_RUNNABLE: OnceCell<flume::Sender<Runnable>> = const { OnceCell::new() };
}

/// Main loop of each thread in the threadpool.
fn main_loop(recv_work: flume::Receiver<UnitOfWork>, recv_stop: async_broadcast::Receiver<()>) {
    let (send_runnable, recv_runnable) = flume::unbounded::<Runnable>();
    SEND_RUNNABLE.with(|tlv| {
        tlv.set(send_runnable.clone())
            .expect("SEND_RUNNABLE already initialized")
    });
    future::block_on(async {
        // A threadpool does one of three things:
        // 1: stop
        let stop_stream = recv_stop.into_stream().map(|()| Event::Stop);
        // 2: Run an existing future that's been re-scheduled for more work
        let old_work_stream = recv_runnable.into_stream().map(Event::OldWork);
        // 3: Start a new unit of work
        let new_work_stream = recv_work.into_stream().map(Event::NewWork);

        let mut event_stream =
            stream::or(stop_stream, stream::or(old_work_stream, new_work_stream));
        while let Some(event) = event_stream.next().await {
            match event {
                Event::Stop => break,
                Event::OldWork(runnable) => {
                    // There is a pending task => run it
                    runnable.run();
                }
                Event::NewWork(query) => {
                    // There is no pending task => add a new one
                    let send_runnable = send_runnable.clone();
                    let (runnable, task) = async_task::spawn_local(
                        async {
                            let UnitOfWork { key, ctx, send } = query;
                            let output = ctx.query_internal(key).await;
                            send.send(output).expect("output send error");
                        },
                        move |runnable| send_runnable.send(runnable).expect("runnable send error"),
                    );
                    task.detach();
                    runnable.run();
                }
            }
        }
    });
}

#[derive(Debug, Copy, Clone)]
pub struct CurrentThreadExecutor;

impl<F> hyper::rt::Executor<F> for CurrentThreadExecutor
where
    F: Future + 'static,
    F::Output: 'static,
{
    fn execute(&self, fut: F) {
        let send_runnable = SEND_RUNNABLE.with(|tlv| {
            tlv.get()
                .expect("SEND_RUNNABLE not initialized for this thread")
                .clone()
        });
        let (runnable, task) = async_task::spawn_local(fut, move |runnable| {
            send_runnable.send(runnable).expect("runnable send error")
        });
        task.detach();
        runnable.run();
    }
}
