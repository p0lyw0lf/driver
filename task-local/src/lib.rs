pub use pastey::paste;

#[macro_export]
macro_rules! task_local {
    ($(static $ident:ident : $ty:ty ;)*) => {$( $crate::paste! {
        mod [<def_ $ident:lower:snake>] {
            use super::*;
            use std::cell::RefCell;
            use std::pin::Pin;
            use std::task::{Context, Poll};

            std::thread_local! {
                /// Thread-local that only gets set while the associated task is actively being
                /// polled.
                static LOCAL: RefCell<Option<$ty>> = const { RefCell::new(None) };
            }

            /// Wrapper future that stores a previous context to the stack, if there is any.
            struct Scoped<F: Future> {
                curr: Option<$ty>,
                fut: F,
            }

            impl<F: Future> Future for Scoped<F> {
                type Output = F::Output;
                fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                    // SAFETY: self stays pinned
                    let this = unsafe { &mut self.get_unchecked_mut() };
                    // SAFETY: because self is pinned, this is pinned
                    let fut = unsafe { Pin::new_unchecked(&mut this.fut) };
                    let curr = &mut this.curr;
                    let mut swap = || LOCAL.with_borrow_mut(|prev| std::mem::swap(curr, prev));

                    swap();
                    let out = fut.poll(cx);
                    swap();

                    out
                }
            }

            impl<F: Future> Scoped<F> {
                /// Call this to get the value out of the scope once you're done polling the
                /// future.
                pub fn take_value(self: Pin<&mut Self>) -> Option<$ty> {
                    // SAFETY: self stays pinned
                    let this = unsafe { &mut self.get_unchecked_mut() };
                    this.curr.take()
                }
            }

            pub struct ScopeBuilder;
            impl ScopeBuilder {
                pub fn scope<F: Future>(&self, value: $ty, fut: F) -> Scoped<F> {
                    Scoped {
                        curr: Some(value),
                        fut,
                    }
                }

                pub fn with<T>(&self, f: impl FnOnce(Option<&$ty>) -> T) -> T {
                    LOCAL.with_borrow(|value| f(value.as_ref()))
                }

                pub fn with_mut<T>(&self, f: impl FnOnce(Option<&mut $ty>) -> T) -> T {
                    LOCAL.with_borrow_mut(|value| f(value.as_mut()))
                }
            }
        }

        static $ident: [<def_ $ident:lower:snake>]::ScopeBuilder = [<def_ $ident:lower:snake>]::ScopeBuilder;
    } )*};
}
