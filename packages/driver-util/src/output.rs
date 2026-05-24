use std::fmt::Debug;

use serde::{Deserialize, Serialize};

use crate::ToHash;

/// A marker trait that represents a collection of traits needed to ensure the output of a query
/// can be used in the ways we need it to.
///
/// In order to make the lifetimes work out, we do need the `Clone` bound unfortunately. If you're
/// passing around large objects, see the driver-db crate for content-addressed objects that
/// satisfy this trait.
pub trait Output: ToHash + Clone + Debug + Send + Serialize + for<'de> Deserialize<'de> {}
impl<T> Output for T where T: ToHash + Clone + Debug + Send + Serialize + for<'de> Deserialize<'de> {}
