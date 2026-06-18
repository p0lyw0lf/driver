//! Standard-library-only implementation of an async task local.
//!
//! See <https://wolfgirl.dev/blog/2026-06-16-async-task-locals-from-scratch/>.

use std::cell::RefCell;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::thread::LocalKey;

/// Container type wraps an [`std::future::Future`], as well as saving/restoring a task-local made
/// available to it.
///
/// Constructed via [`ScopeBuilder::scope`].
pub struct Scoped<F, T: 'static> {
    /// Thread-Local Storage that holds the current task-local value.
    tls: &'static LocalKey<RefCell<Option<T>>>,
    /// When the future IS NOT being polled: the value we want to store is inside `curr`.
    ///
    /// When the future IS being polled: the previous value stored inside `curr`, if we are running
    /// inside another [`Scoped`].
    curr: Option<T>,
    /// The future we're wrapping.
    fut: F,
}

/// Panic-safe wrapper that helps us call a function on drop.
struct Defer<'a, F: FnMut()> {
    f: &'a mut F,
}

impl<'a, F: FnMut()> Drop for Defer<'a, F> {
    fn drop(&mut self) {
        (self.f)();
    }
}

impl<F: Future, T: 'static> Future for Scoped<F, T> {
    type Output = F::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: `this` never moves out its value, so `self` stays pinned.
        let this = unsafe { &mut self.get_unchecked_mut() };
        // SAFETY: because `self` is still pinned, so is `fut`.
        let fut = unsafe { Pin::new_unchecked(&mut this.fut) };
        // The swap only ever moves values _stored inside_ `self`;
        // it doesn't change the location of `self` directly.
        let curr = &mut this.curr;
        let mut swap = || this.tls.with_borrow_mut(|prev| std::mem::swap(curr, prev));

        // Swap in the value from "the stack" (`this.curr`) into "global memory" (`this.tls`) while
        // polling.
        swap();
        // Swap the value from "global memory" back onto "the stack" to save it for when we get
        // polled later.
        // Do this either on normal function exit OR on panic.
        let _swap_on_drop = Defer { f: &mut swap };

        fut.poll(cx)
    }
}

impl<F, T: 'static> Scoped<F, T> {
    /// Call this to get the value out of the scope once you're done polling the
    /// future.
    ///
    /// Usage:
    /// ```rust
    /// async_task_local::task_local! {
    ///     static SOME_GLOBAL: usize;
    /// }
    ///
    /// # #[tokio::main]
    /// # async fn main() {
    /// let scoped = SOME_GLOBAL.scope(67, async {
    ///     SOME_GLOBAL.with_mut(|v| v.map(|v| { *v += 2; }));
    /// });
    /// let mut scoped = std::pin::pin!(scoped);
    /// scoped.as_mut().await;
    /// assert_eq!(scoped.take_value(), Some(69));
    /// # }
    /// ```
    pub fn take_value(self: Pin<&mut Self>) -> Option<T> {
        // SAFETY: self stays pinned
        let this = unsafe { &mut self.get_unchecked_mut() };
        this.curr.take()
    }
}

/// Helper struct for constructing [`Scoped`] futures for a given Thread-Local Storage.
pub struct ScopeBuilder<T: 'static> {
    tls: &'static LocalKey<RefCell<Option<T>>>,
}

impl<T: 'static> ScopeBuilder<T> {
    /// Create a new task wrapper builder for the given Thread-Local Storage
    ///
    /// Usually called from inside the [`task_local!`] macro.
    pub const fn new(tls: &'static LocalKey<RefCell<Option<T>>>) -> Self {
        Self { tls }
    }

    /// Given the Thread-Local Storage provided at builder creation, construct a [`Scoped`] future
    /// that will expose the given `value`.
    pub fn scope<F: Future>(&self, value: T, fut: F) -> Scoped<F, T> {
        Scoped {
            tls: self.tls,
            curr: Some(value),
            fut,
        }
    }

    /// Read from the Thread-Local Storage, which will be `Some` if we are inside a [`Scoped`]
    /// future. Takes a callback to ensure the lifetimes work out.
    pub fn with<V>(&self, f: impl FnOnce(Option<&T>) -> V) -> V {
        self.tls.with_borrow(|value| f(value.as_ref()))
    }

    /// Read/write the value in Thread-Local Storage, which will be `Some` if we are inside a
    /// [`Scoped`] future. Takes a callback to ensure the lifetimes work out.
    pub fn with_mut<V>(&self, f: impl FnOnce(Option<&mut T>) -> V) -> V {
        self.tls.with_borrow_mut(|value| f(value.as_mut()))
    }
}

/// Convenience macro, similar to `tokio::task_local!`, that defines Thread-Local storage and an
/// associated [`ScopeBuilder`] at the same time.
///
/// Usage:
/// ```rust
/// async_task_local::task_local! {
///     static SOME_GLOBAL: usize;
/// }
///
/// # #[tokio::main]
/// # async fn main() {
/// let mut values = vec![];
/// // Accumulate the current value in `SOME_GLOBAL` into the vec
/// let mut get = || SOME_GLOBAL.with(|v| v.map(|v| values.push(*v)));
///
/// get();    
/// SOME_GLOBAL.scope(5138008, async {
///     get();
///     SOME_GLOBAL.scope(69, async {
///         get();
///         SOME_GLOBAL.scope(42, async {
///             get();
///         }).await;
///         get();
///     }).await;
///     get();
/// }).await;
/// get();
///
/// assert_eq!(values, [5138008, 69, 42, 69, 5138008]);
/// # }
// ```
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
