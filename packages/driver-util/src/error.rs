use std::fmt::Display;

use serde::{Deserialize, Serialize};

/// A very simple arbitrary error wrapper that just serializes everything to a String. Used in
/// place of anyhow so that we can clone it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Error(String);

impl Error {
    pub fn new(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<E> From<E> for Error
where
    E: std::error::Error,
{
    fn from(err: E) -> Self {
        let mut msg = err.to_string();
        // It seems many error implementations don't include source data as part of their message,
        // and instead we need to go down the stack manually
        let mut err = err.source();
        while let Some(e) = err {
            msg.push_str(&format!("\n\t{e}"));
            err = e.source();
        }
        Self(msg)
    }
}

// Can't be done, though I think that's OK?
// impl std::error::Error for Error {}

/// A wrapper type around [`Error`] that allows it to be used as an [`std::error::Error`].
/// Not Serialize/Deserialize to discourage storing it anywhere; it SHOULD only be used for
/// reporting.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StdError(Error);

impl From<Error> for StdError {
    fn from(value: Error) -> Self {
        Self(value)
    }
}

impl Display for StdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl std::error::Error for StdError {}
