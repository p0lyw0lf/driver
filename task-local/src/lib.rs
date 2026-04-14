use std::cell::RefCell;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::thread::LocalKey;

/// Wrapper future that stores a previous context to the stack, if there is any.
pub struct Scoped<F: Future, T: 'static> {
    tls: &'static LocalKey<RefCell<Option<T>>>,
    curr: Option<T>,
    fut: F,
}

impl<F: Future, T: 'static> Future for Scoped<F, T> {
    type Output = F::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: self stays pinned
        let this = unsafe { &mut self.get_unchecked_mut() };
        // SAFETY: because self is pinned, this is pinned
        let fut = unsafe { Pin::new_unchecked(&mut this.fut) };
        let curr = &mut this.curr;
        let mut swap = || this.tls.with_borrow_mut(|prev| std::mem::swap(curr, prev));

        swap();
        let out = fut.poll(cx);
        swap();

        out
    }
}

impl<F: Future, T: 'static> Scoped<F, T> {
    /// Call this to get the value out of the scope once you're done polling the
    /// future.
    pub fn take_value(self: Pin<&mut Self>) -> Option<T> {
        // SAFETY: self stays pinned
        let this = unsafe { &mut self.get_unchecked_mut() };
        this.curr.take()
    }
}

pub struct ScopeBuilder<T: 'static> {
    tls: &'static LocalKey<RefCell<Option<T>>>,
}
impl<T: 'static> ScopeBuilder<T> {
    pub const fn new(tls: &'static LocalKey<RefCell<Option<T>>>) -> Self {
        Self { tls }
    }

    pub fn scope<F: Future>(&self, value: T, fut: F) -> Scoped<F, T> {
        Scoped {
            tls: self.tls,
            curr: Some(value),
            fut,
        }
    }

    pub fn with<V>(&self, f: impl FnOnce(Option<&T>) -> V) -> V {
        self.tls.with_borrow(|value| f(value.as_ref()))
    }

    pub fn with_mut<V>(&self, f: impl FnOnce(Option<&mut T>) -> V) -> V {
        self.tls.with_borrow_mut(|value| f(value.as_mut()))
    }
}

#[macro_export]
macro_rules! task_local {
    ($(static $ident:ident : $ty:ty ;)*) => {$(
        static $ident: $crate::ScopeBuilder<$ty> = {
            std::thread_local! {
                static LOCAL: std::cell::RefCell<Option<$ty>> = const { std::cell::RefCell::new(None) };
            }
            $crate::ScopeBuilder::new(&LOCAL)
        };
    )*}
}
