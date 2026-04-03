pub use pastey::paste;

#[macro_export]
macro_rules! task_local {
    ($(static $ident:ident : $ty:ty ;)*) => {$( $crate::paste! {
        mod [<def_ $ident:snake>] {
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
                    // SAFETY: self is pinned
                    let fut = unsafe { Pin::new_unchecked(&mut self.get_unchecked_mut().fut) };
                    // SAFETY: self doesn't move as a result of this
                    let curr = unsafe { &mut self.get_unchecked_mut().curr };
                    let mut swap = || LOCAL.with_borrow_mut(|prev| std::mem::swap(curr, prev));

                    swap();
                    let out = fut.poll(cx);
                    swap();

                    out
                }
            }

            impl<F: Future> Scoped<F> {
                pub fn take_value(mut self) -> $ty {
                    // SAFETY: by usage, we cannot ever have a `None` value, except temporary
                    // (while not owned) in the middle of executing
                    self.curr.take().expect("scope context somehow none")
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
                    LOCAL.with_borrow(|value| f(value))
                }
            }
        }

        static $ident: [<def_ $ident:snake>]::ScopeBuilder = [<def_ $ident:snake>]::ScopeBuilder;
    } )*};
}
