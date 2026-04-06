use std::{fmt::Display, ops::Deref};

use hyper_util::client::legacy::Client as HyperClient;
use serde::{Deserialize, Serialize, de::Error};

use crate::engine::executor::CurrentThreadExecutor;

pub type Client = HyperClient<
    hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>,
    http_body_util::Empty<hyper::body::Bytes>,
>;
pub fn default_client() -> Client {
    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_native_roots()
        .expect("no native root CA certificates found")
        .https_or_http()
        .enable_http1()
        .build();
    HyperClient::builder(CurrentThreadExecutor).build(https)
}

pub static USER_AGENT: &str = concat!(
    "hyper (",
    env!("CARGO_PKG_NAME"),
    " ",
    env!("CARGO_PKG_VERSION"),
    ") (+https://github.com/p0lyw0lf/driver)",
);

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
        let uri = hyper::Uri::try_from(s).map_err(|err| D::Error::custom(err))?;
        Ok(Uri(uri))
    }
}
