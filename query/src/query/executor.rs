use async_task::Runnable;
use futures_lite::future;

use crate::query::{
    context::{AnyOutput, Producer, QueryContext},
    key::QueryKey,
};

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
pub struct Executor {
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
    pub fn start() -> Executor {
        let (send_work, recv_work) = flume::unbounded();
        let (send_stop, recv_stop) = async_broadcast::broadcast(1);

        for _ in 0..num_cpus::get() {
            let recv_work = recv_work.clone();
            let recv_stop = recv_stop.clone();
            std::thread::spawn(|| main_loop(recv_work, recv_stop));
        }

        Self {
            send_work,
            send_stop,
        }
    }

    pub(crate) async fn execute(&self, key: QueryKey, ctx: QueryContext) -> AnyOutput {
        let (send, recv) = oneshot::async_channel();
        let query = UnitOfWork { key, ctx, send };
        self.send_work.send(query).expect("query send error");
        recv.await.expect("output receive error")
    }

    pub fn stop(self) {
        let _ = self
            .send_stop
            .broadcast_blocking(())
            .expect("stop send error");
    }
}

/// The type of event that can be received on each thread's main loop
enum Event {
    OldWork(Runnable),
    NewWork(UnitOfWork),
    Stop,
}

/// Main loop of each thread in the threadpool.
fn main_loop(recv_work: flume::Receiver<UnitOfWork>, mut recv_stop: async_broadcast::Receiver<()>) {
    let (send_runnable, recv_runnable) = flume::unbounded::<Runnable>();
    future::block_on(async {
        loop {
            // A threadpool does one of three things:
            // 1: stop
            let stop_fut = async {
                let () = recv_stop.recv_direct().await.expect("stop receive error");
                Event::Stop
            };
            // 2: Run an existing future that's been re-scheduled for more work
            let old_work_fut = async {
                let runnable = recv_runnable
                    .recv_async()
                    .await
                    .expect("runnable receive error");
                Event::OldWork(runnable)
            };
            // 3: Start a new unit of work
            let new_work_fut = async {
                let work = recv_work.recv_async().await.expect("work receive error");
                Event::NewWork(work)
            };
            // NOTE: All of the above futures are cancel-safe! They drop their place in the
            // receiving line when the future object itself is dropped, no matter if
            // it's been polled to completion or now.
            // Prefer higher-priority futures over lower priority ones with the `future::or`
            // function, which works like a poor woman's `select!` macro.
            match future::block_on(future::or(stop_fut, future::or(old_work_fut, new_work_fut))) {
                Event::Stop => break,
                Event::OldWork(runnable) => {
                    // There is a pending task => run it
                    runnable.run();
                }
                Event::NewWork(query) => {
                    // There is no pending task => add a new one
                    let send_runnable = send_runnable.clone();
                    let (runnable, _) = async_task::spawn_local(
                        async move {
                            let UnitOfWork { key, ctx, send } = query;
                            let output = key.query(&ctx).await;
                            send.send(output).expect("output send error");
                        },
                        move |runnable| send_runnable.send(runnable).expect("runnable send error"),
                    );
                    runnable.run();
                }
            }
        }
    });
}
