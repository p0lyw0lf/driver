use std::collections::HashMap;

use dashmap::DashMap;
use jiff::fmt::temporal::DateTimeParser;
use jiff::{Span, Timestamp, ToSpan};
use reqwest::blocking::Client;
use reqwest::header::{ETAG, EXPIRES, HeaderMap, HeaderValue, IF_MODIFIED_SINCE, IF_NONE_MATCH};
use reqwest::{Url, header::CACHE_CONTROL};
use serde::{Deserialize, Serialize};

use crate::db::object::Object;
use crate::db::object::Objects;

/// A store for all URLs that have been fetched remotely. Maps a URL to an object hash and
/// expiration time, if present on the fetched headers.
#[derive(Debug)]
pub struct RemoteObjects {
    // TODO: eventually I'd like to have this be an async client, but porting all my code to be
    // async seems a little sus atm :)
    client: Client,
    cache: DashMap<Url, RemoteObject>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteObject {
    /// The stored object we fetched.
    object: Object,
    /// The time at which we fetched the object.
    fetched: Timestamp,
    /// How long after `fetched` can we continue to treat the object as "fresh" (don't need to
    /// fetch again)? Calculated according to https://httpwg.org/specs/rfc9111.html#calculating.freshness.lifetime,
    /// based on the HTTP responose headers.
    freshness_lifetime: Span,
    /// When submitting to the cache server, we provide an ETag header so it can say "not modified"
    /// to short-circuit make us not have to download as much data
    etag: Option<Vec<u8>>,
}

impl RemoteObject {
    /// Returns whether the object is still fresh at the time of the call.
    fn is_fresh(&self) -> bool {
        let now = Timestamp::now();
        let since_then = now - self.fetched;
        matches!(
            self.freshness_lifetime.compare(since_then),
            Ok(std::cmp::Ordering::Greater)
        )
    }
}

impl RemoteObjects {
    fn default_client() -> Client {
        Client::builder()
            .user_agent(format!(
                "reqwest ({} {}) (+https://github.com/p0lyw0lf/driver)",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION")
            ))
            .gzip(true)
            .zstd(true)
            .build()
            .expect("could not build HTTP client")
    }

    fn default_freshness() -> Span {
        1.day()
    }

    /// Fetches a remote URL and adds it to the local store if not present or too stale.
    /// If the URL is present in the cache and still fresh, uses that instead of fetching.
    fn fetch(&self, objects: &Objects, url: Url) -> crate::Result<RemoteObject> {
        // If there is a fresh object in the cache, just use that
        if let Some(remote_object) = self.cache.get(&url)
            && remote_object.is_fresh()
        {
            return Ok(remote_object.clone());
        }

        // Otherwise, we need to fetch the URL.
        let mut req = self.client.get(url.clone());
        if let Some(remote_object) = self.cache.get(&url) {
            // TODO: See
            // https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/If-Modified-Since
            // for the format of this. kinda cursed if you ask me tbh
            req = req.header(
                IF_MODIFIED_SINCE,
                "TODO: correctly format this from remote_object.fetched",
            );
            if let Some(etag) = &remote_object.etag {
                req = req.header(IF_NONE_MATCH, HeaderValue::from_bytes(etag)?);
            }
        }
        let resp = req.send()?;
        let status = resp.status();
        if !status.is_success() {
            // TODO: check for 304 status. should probably also hold the lock on remote_object
            // until here if I can.
            return Err(crate::Error::new(
                status.canonical_reason().unwrap_or("unknown response code"),
            ));
        }

        let fetched = Timestamp::now();
        let freshness_lifetime = Self::calculate_freshness_lifetime(resp.headers(), fetched)
            .unwrap_or_else(|e| {
                // Log the error, then continue with default freshness, since the server _did_ give
                // us a response after all.
                tracing::warn!("getting freshness lifetime: {e}");
                Self::default_freshness()
            });
        let etag = resp
            .headers()
            .get(ETAG)
            .map(|header| header.as_bytes().to_owned());

        let body = resp.bytes()?;
        let object = objects.store(body.into());

        Ok(RemoteObject {
            object,
            fetched,
            freshness_lifetime,
            etag,
        })
    }

    /// Runs the algorithm described at https://httpwg.org/specs/rfc9111.html#rfc.section.4.2.1
    fn calculate_freshness_lifetime(
        headers: &HeaderMap,
        fetched: Timestamp,
    ) -> crate::Result<Span> {
        if let Some(cache_control) = headers.get(CACHE_CONTROL) {
            let cache_control = cache_control.to_str()?;
            let directives = cache_control
                .split(",")
                .map(str::trim)
                .map(|directive| match directive.split_once("=") {
                    None => (directive.to_string().to_ascii_lowercase(), None),
                    Some((directive, value)) => (
                        directive.to_string().to_ascii_lowercase(),
                        Some(value.to_string()),
                    ),
                })
                .collect::<HashMap<_, _>>();

            if directives.contains_key("no-cache") || directives.contains_key("no-store") {
                // Server says we shouldn't cache this value, return a zero-time span
                return Ok(0.seconds());
            }
            if let Some(Some(seconds)) = directives.get("s-maxage") {
                let seconds: i64 = seconds.parse()?;
                return Ok(seconds.seconds());
            }
            if let Some(Some(seconds)) = directives.get("max-age") {
                let seconds: i64 = seconds.parse()?;
                return Ok(seconds.seconds());
            }
        }

        if let Some(expires) = headers.get(EXPIRES) {
            static PARSER: DateTimeParser = DateTimeParser::new();
            let expires = PARSER.parse_timestamp(expires)?;
            return Ok(expires - fetched);
        }

        // Use a reasonable default if the remote doesn't provide caching headers (or just etags,
        // which I don't support currently because I don't want to make _any_ network requests if
        // the cache is fresh)
        Ok(Self::default_freshness())
    }
}

impl Serialize for RemoteObjects {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Only serialize the cache; client is built manually
        self.cache.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RemoteObjects {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let client = Self::default_client();
        let cache = DashMap::deserialize(deserializer)?;
        Ok(Self { client, cache })
    }
}

impl Default for RemoteObjects {
    fn default() -> Self {
        Self {
            client: Self::default_client(),
            cache: Default::default(),
        }
    }
}
