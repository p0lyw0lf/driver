//! Various utility datastructures that are used in the driver collection of crates.

mod error;
pub use error::Error;
pub type Result<T> = std::result::Result<T, Error>;

mod serde;
pub use serde::SerializedMap;

mod key;
pub use key::Key;

mod output;
pub use output::Output;
