use std::any::Any;
use std::cell::OnceCell;
use std::pin::Pin;
use std::thread::JoinHandle;

use async_task::Runnable;
use futures_concurrency::stream::IntoStream;
use futures_lite::future;
use futures_lite::stream::{self, StreamExt as _};

#[cfg(feature="hyper")]
mod hyper;

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
    /// All the threads in the threadpool.
    threads: Vec<JoinHandle<()>>,
    /// The sending end of a channel that lets us spawn new queries onto the threadpool.
    send_work: flume::Sender<UnitOfWork>,
    /// A broadcast channel to let us stop the threadpool
    send_stop: async_broadcast::Sender<()>,
    // TODO: The sending end of a channel that lets us send things back to the entire threadpool
    // if they are `Send`.
    // global_send_runnable: flume::Sender<Runnable>,
}

type BoxedFuture = Box<dyn Future<Output = Box<dyn Any>>>;

/// A single "unit of work" (lmao at the name) that the executor keeps track of. Multiple
/// corredponding to a single key/ctx can be in-flight at the same time, but only the first one
/// will actually run any "real" computation (thanks to a locking mechanism).
struct UnitOfWork {
    fut: Pin<BoxedFuture>,
    send: oneshot::Sender<Box<dyn Any>>,
}

impl Executor {
    /// MUST NOT be run in an async context.
    pub fn start() -> Executor {
        let (send_work, recv_work) = flume::unbounded();
        let (send_stop, recv_stop) = async_broadcast::broadcast(1);

        let n = num_cpus::get();
        let threads = (0..n)
            .map(|_| {
                let recv_work = recv_work.clone();
                let recv_stop = recv_stop.clone();
                std::thread::spawn(move || main_loop(recv_work, recv_stop))
            })
            .collect();

        Self {
            threads,
            send_work,
            send_stop,
        }
    }

    /// Pushes a new unit of work onto the global queue, where it then be pinned to the first
    /// thread that starts executing it.
    pub async fn execute<F>(&self, fut: F) -> F::Output
    where
        F: Future,
        F::Output: Send,
    {
        let (send, recv) = oneshot::channel();
        let query = UnitOfWork {
            fut: Box::into_pin(
                Box::new(async { Box::new(fut.await) as Box<dyn Any> }) as BoxedFuture
            ),
            send,
        };
        self.send_work.send(query).expect("query send error");
        *recv
            .await
            .expect("output receive error")
            .downcast()
            .expect("output type error")
    }

    /// MUST NOT be run in an async context.
    pub fn stop(self) {
        // Tell all threads to stop running
        let _ = self
            .send_stop
            .broadcast_blocking(())
            .expect("stop send error");

        // Wait for all threads to finish running
        for thread in self.threads.into_iter() {
            thread.join().expect("thread join error");
        }
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
        // TODO: relax this restriction
        tlv.set(send_runnable.clone())
            .expect("SEND_RUNNABLE already initialized; there can only be one Executor running for the duration of a program.")
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
                            let UnitOfWork { fut, send } = query;
                            send.send(fut.await).expect("output send error");
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

/// If an `Executor` is running, use this to run further futures on the current thread.
#[derive(Debug, Copy, Clone)]
pub struct CurrentThreadExecutor;

impl CurrentThreadExecutor {
    pub fn spawn<F>(&self, fut: F)
    where
        F: Future,
    {
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
