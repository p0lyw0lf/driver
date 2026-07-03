//! Various utility datastructures that are used in the driver collection of crates.

mod error;
pub use error::Error;
pub use error::StdError;
pub type Result<T> = std::result::Result<T, Error>;

mod serde;
pub use serde::SerializedMap;

mod key;
pub use key::Key;

mod blob;
pub use blob::Blob;
pub use blob::BlobTrace;

mod output;
pub use output::Output;

mod write_output;
pub use write_output::WriteOutput;
