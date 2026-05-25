use crate::{CurrentThreadExecutor, Executor};

impl<F> hyper::rt::Executor<F> for Executor
where
    F: Future,
    F::Output: Send,
{
    fn execute(&self, fut: F) {
        todo!()
    }
}

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
