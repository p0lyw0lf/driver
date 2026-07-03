//! A "Producer" is a Key that has been annotated with a function that lets it do incremental
//! computation, and a "Context" is a structure that holds all the state required to track the
//! incremental computations. To that end, we've defined a couple of other things too:
//!
//! - `ProducerBase`, which declares the singular output type a `Producer` can output.
//! - `query`, which allows producers/external code to run an incremental computation.
//! - A whole host of macros (`key!`, `producer!`, `query_key!`) to make writing these easier.
//!
//! Example usage:
//!
//! ```rust
//! use driver_engine::{Context, query};
//!
//! driver_engine::key!(
//!     #[input=|_| false]
//!     struct Fib(u32);
//! );
//! driver_engine::no_objects!(Fib);
//! driver_engine::producer!(Fib(self, ctx) where [Fib] -> u32 {
//!     let n = self.0;
//!     if n == 0 || n == 1 {
//!         return 1;
//!     }
//!
//!     let n_1 = query(ctx, Fib(n-1)).await;
//!     let n_2 = query(ctx, Fib(n-2)).await;
//!
//!     n_1 + n_2
//! });
//!
//! driver_engine::query!(Key { Fib } with Output);
//!
//! let ctx = Context::<Key>::create_empty_root_for_testing_only();
//! let output = futures_lite::future::block_on(query(&ctx, Fib(10)));
//! assert_eq!(output, 89);
//!
//! impl std::fmt::Display for Fib {
//!     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//!         write!(f, "Fib({})", self.0)
//!     }
//! }
//! ```

mod context;
pub use context::Context;
pub use context::Hooks;

mod producer;
pub use producer::Downcastable;
pub use producer::Producer;
pub use producer::ProducerBase;
pub use producer::query;

/// Re-export for convenience
pub use driver_db::Options;
pub use driver_db::Uri;
pub use driver_util::Object;
pub use driver_util::ObjectTrace;
pub use driver_util::key;
pub use driver_util::no_objects;
pub use driver_util::object_trace;
