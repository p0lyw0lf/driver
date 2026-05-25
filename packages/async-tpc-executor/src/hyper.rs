use super::{CurrentThreadExecutor, Executor};

impl<F> hyper::rt::Executor<F> for Executor
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    fn execute(&self, fut: F) {
        let _ = self.spawn_unpinned(fut);
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
