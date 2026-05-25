use super::{CurrentThreadExecutor, Executor, SEND_RUNNABLE};

impl<F> hyper::rt::Executor<F> for Executor
where
    F: Future,
    F::Output: Send,
{
    fn execute(&self, fut: F) {
        // This should spawn a Runnable onto a global queue, instead of boxing the future a second
        // time.
        // Unfortunately, the double-boxing I'm having to do (first box for the future sent on the
        // work queue, second box for the runnable queue) is seemingly unavoidable, because
        // spawn_local runnables can't be sent. TODO figure this out later
        todo!()
    }
}

impl<F> hyper::rt::Executor<F> for CurrentThreadExecutor
where
    F: Future + 'static,
    F::Output: 'static,
{
    fn execute(&self, fut: F) {
        Self.spawn(fut)
    }
}
