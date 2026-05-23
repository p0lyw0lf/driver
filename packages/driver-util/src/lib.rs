//! Various utility datastructures that are used in the driver collection of crates.

mod error;
pub use error::Error;
pub type Result<T> = std::result::Result<T, Error>;

pub mod serde;

mod to_hash;
pub use to_hash::ToHash;
