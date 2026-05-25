use std::pin::Pin;
use std::thread::JoinHandle;

use async_task::Runnable;
use futures_concurrency::stream::IntoStream;
use futures_lite::future;
use futures_lite::stream::{self, StreamExt as _};

#[cfg(feature = "hyper")]
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
}

type BoxedFn = Box<dyn (FnOnce() -> Pin<BoxedFuture>) + Send>;
type BoxedFuture = Box<dyn Future<Output = ()>>;
type SendBoxedFuture = Box<dyn Future<Output = ()> + Send>;

/// A single "unit of work" (lmao at the name) that the executor keeps track of. Multiple
/// corredponding to a single key/ctx can be in-flight at the same time, but only the first one
/// will actually run any "real" computation (thanks to a locking mechanism).
enum UnitOfWork {
    /// Runnables created with this will stay "pinned" to whatever thread picks them up first.
    Pinned(BoxedFn),
    /// Runnables created with this will be "unpinned", available to run on whatever thread is available.
    Unpinned(Pin<SendBoxedFuture>),
}

impl Executor {
    /// MUST NOT be run in an async context.
    pub fn start() -> Executor {
        let (send_work, recv_work) = flume::unbounded();
        let (send_unpinned_runnable, recv_unpinned_runnable) = flume::unbounded();
        let (send_stop, recv_stop) = async_broadcast::broadcast(1);

        let n = num_cpus::get();
        let threads = (0..n)
            .map(|_| {
                let recv_work = recv_work.clone();
                let send_unpinned_runnable = send_unpinned_runnable.clone();
                let recv_unpinned_runnable = recv_unpinned_runnable.clone();
                let recv_stop = recv_stop.clone();
                std::thread::spawn(move || {
                    main_loop(
                        recv_work,
                        send_unpinned_runnable,
                        recv_unpinned_runnable,
                        recv_stop,
                    )
                })
            })
            .collect();

        Self {
            threads,
            send_work,
            send_stop,
        }
    }

    fn spawn_pinned<F, Fut>(&self, f: F) -> oneshot::Receiver<Fut::Output>
    where
        F: (FnOnce() -> Fut) + Send + 'static,
        Fut: Future + 'static,
        Fut::Output: Send + 'static,
    {
        let (send, recv) = oneshot::channel();
        let work = UnitOfWork::Pinned(Box::new(move || {
            let fut = f();
            Box::into_pin(Box::new(async {
                let output = fut.await;
                send.send(output).expect("pinned output send error");
            }) as BoxedFuture)
        }));
        self.send_work.send(work).expect("pinned work send error");
        recv
    }

    /// Pushes a new unit of work onto the global queue, where it then be pinned to the first
    /// thread that starts executing it.
    pub async fn execute_pinned<F, Fut>(&self, f: F) -> Fut::Output
    where
        F: (FnOnce() -> Fut) + Send + 'static,
        Fut: Future + 'static,
        Fut::Output: Send + 'static,
    {
        self.spawn_pinned(f)
            .await
            .expect("pinned output receive error")
    }

    fn spawn_unpinned<Fut>(&self, fut: Fut) -> oneshot::Receiver<Fut::Output>
    where
        Fut: Future + Send + 'static,
        Fut::Output: Send + 'static,
    {
        let (send, recv) = oneshot::channel();
        let work = UnitOfWork::Unpinned(Box::into_pin(Box::new(async {
            let output = fut.await;
            send.send(output).expect("unpinned output send error");
        }) as SendBoxedFuture));
        self.send_work.send(work).expect("unpinned work send error");
        recv
    }

    /// Pushes a new unit of work onto the global queue, where any available thread can execute it.
    pub async fn execute_unpinned<Fut>(&self, fut: Fut) -> Fut::Output
    where
        Fut: Future + Send + 'static,
        Fut::Output: Send + 'static,
    {
        self.spawn_unpinned(fut)
            .await
            .expect("unpinned output receive error")
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

/// Main loop of each thread in the threadpool.
fn main_loop(
    recv_work: flume::Receiver<UnitOfWork>,
    send_unpinned_runnable: flume::Sender<Runnable>,
    recv_unpinned_runnable: flume::Receiver<Runnable>,
    recv_stop: async_broadcast::Receiver<()>,
) {
    let (send_pinned_runnable, recv_pinned_runnable) = flume::unbounded::<Runnable>();
    future::block_on(async {
        // A threadpool does one of three things:
        // 1: stop
        let stop_stream = recv_stop.into_stream().map(|()| Event::Stop);
        // 2.1: Run an existing future from the "local" poll that's been re-scheduled for more work
        let pinned_runnable_stream = recv_pinned_runnable.into_stream().map(Event::OldWork);
        // 2.2: Run an existing future from the "global" pool (lower priority)
        let unpinned_runnable_stream = recv_unpinned_runnable.into_stream().map(Event::OldWork);
        // 3: Start a new unit of work
        let new_work_stream = recv_work.into_stream().map(Event::NewWork);

        let mut event_stream = stream::or(
            stop_stream,
            stream::or(
                stream::race(pinned_runnable_stream, unpinned_runnable_stream),
                new_work_stream,
            ),
        );
        while let Some(event) = event_stream.next().await {
            match event {
                Event::Stop => break,
                Event::OldWork(runnable) => {
                    // There is a pending task => run it
                    runnable.run();
                }
                Event::NewWork(UnitOfWork::Pinned(mk_fut)) => {
                    // There is no pending task => add a new one
                    let send_pinned_runnable = send_pinned_runnable.clone();
                    let (runnable, task) = async_task::spawn_local((mk_fut)(), move |runnable| {
                        send_pinned_runnable
                            .send(runnable)
                            .expect("pinned runnable send error")
                    });
                    task.detach();
                    runnable.run();
                }
                Event::NewWork(UnitOfWork::Unpinned(fut)) => {
                    let send_unpinned_runnable = send_unpinned_runnable.clone();
                    let (runnable, task) = async_task::spawn(fut, move |runnable| {
                        send_unpinned_runnable
                            .send(runnable)
                            .expect("unpinned runnable send error")
                    });
                    task.detach();
                    runnable.run();
                }
            }
        }
    });
}
