use std::fmt::Display;
use std::ops::Deref;

use serde::{Deserialize, Serialize, de::Error as _};

/// Newtype in order to get Serialize/Deserialize, and PartialOrd/Ord
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct Uri(pub hyper::Uri);

impl From<Uri> for hyper::Uri {
    fn from(value: Uri) -> Self {
        value.0
    }
}

impl Deref for Uri {
    type Target = hyper::Uri;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Display for Uri {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl PartialOrd for Uri {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Uri {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let a = self.0.to_string();
        let b = other.0.to_string();
        a.cmp(&b)
    }
}

impl Serialize for Uri {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for Uri {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = <&'_ str>::deserialize(deserializer)?;
        let uri = hyper::Uri::try_from(s).map_err(D::Error::custom)?;
        Ok(Uri(uri))
    }
}
