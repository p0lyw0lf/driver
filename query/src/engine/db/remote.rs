use http_body_util::BodyExt;
use hyper::Response;
use hyper::body::Incoming;
use hyper::header::{
    CACHE_CONTROL, ETAG, EXPIRES, HOST, IF_MODIFIED_SINCE, IF_NONE_MATCH, USER_AGENT,
};
use hyper::{HeaderMap, StatusCode, header::HeaderValue};
use jiff::fmt::temporal::DateTimeParser;
use jiff::{Span, Timestamp, ToSpan};
use serde::{Deserialize, Serialize};

use crate::engine::db::http_client::{Client, default_client};
use crate::engine::db::object::{Object, Objects};
use crate::serde::SerializedMap;

pub use crate::engine::db::http_client::Uri;

/// A store for all URLs that have been fetched remotely. Maps a URL to an object hash and
/// expiration time, if present on the fetched headers.
#[derive(Debug, Serialize, Deserialize)]
pub struct RemoteObjects {
    #[serde(skip, default = "crate::engine::db::http_client::default_client")]
    client: Client,
    cache: SerializedMap<Uri, RemoteObject>,
}

impl Default for RemoteObjects {
    fn default() -> Self {
        Self {
            client: default_client(),
            cache: Default::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteObject {
    /// The stored object we fetched.
    pub object: Object,
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
    /// Fetches a remote URL and adds it to the local store if not present or too stale.
    /// If the URL is present in the cache and still fresh, uses that instead of fetching.
    pub async fn fetch(&self, objects: &Objects, uri: Uri) -> crate::Result<RemoteObject> {
        let (req, why) = {
            // Limit lifetime of the remote object that we use to build the request
            let remote_object = self.cache.get_async(&uri).await;
            if let Some(ref remote_object) = remote_object
                && remote_object.is_fresh()
            {
                // If there is a fresh object in the cache, just use that
                return Ok((*remote_object).clone());
            }

            // Otherwise, we need to fetch the URL.
            let mut req = hyper::Request::get(uri.clone())
                .header(USER_AGENT, crate::engine::db::http_client::USER_AGENT)
                .header(HOST, uri.host().expect("no host"));
            if let Some(ref remote_object) = remote_object {
                req = req.header(
                    IF_MODIFIED_SINCE,
                    format_header_date(remote_object.fetched)?,
                );
                if let Some(etag) = &remote_object.etag {
                    req = req.header(IF_NONE_MATCH, HeaderValue::from_bytes(etag)?);
                }
            }

            let why = match remote_object {
                None => "not fetched",
                Some(_) => "stale",
            };

            (req, why)
        };

        let req = req.body(http_body_util::Empty::<hyper::body::Bytes>::new())?;
        println!("[{}] fetching: {}", why, uri);
        let resp: Response<Incoming> = self.client.request(req).await?;
        let status = resp.status();
        if !status.is_success() {
            if status == StatusCode::NOT_MODIFIED {
                // Cache thinks the object we have locally is still fresh, keep it around and
                // update the headers.
                let headers = ResponseHeaders::from_headers(resp.headers());
                let remote_object = self.cache.get_async(&uri).await.ok_or_else(|| {
                    crate::Error::new("server returned 304, but object not found in cache")
                })?;
                return Ok(headers.with_object(remote_object.object.clone()));
            }
            // Otherwise, the error is unexpected
            return Err(crate::Error::new(
                status.canonical_reason().unwrap_or("unknown response code"),
            ));
        }

        let headers = ResponseHeaders::from_headers(resp.headers());

        let body = resp.into_body();
        let body = body.collect().await?.to_bytes();
        let object = objects.store(body.into());

        let remote_object = headers.with_object(object);
        let _ = self.cache.upsert_async(uri, remote_object.clone()).await;

        Ok(remote_object)
    }
}

/// The part of RemoteObject that can be populated from the response headers we get
struct ResponseHeaders {
    fetched: Timestamp,
    freshness_lifetime: Span,
    etag: Option<Vec<u8>>,
}

impl ResponseHeaders {
    fn with_object(self, object: Object) -> RemoteObject {
        let Self {
            fetched,
            freshness_lifetime,
            etag,
        } = self;
        RemoteObject {
            object,
            fetched,
            freshness_lifetime,
            etag,
        }
    }

    /// If the server doesn't support cache tracking, how long should we cache anyways?
    fn default_freshness() -> Span {
        1.day()
    }

    fn from_headers(headers: &HeaderMap) -> Self {
        let fetched = Timestamp::now();
        let freshness_lifetime = Self::calculate_freshness_lifetime(headers, fetched)
            .unwrap_or_else(|e| {
                // Log the error, then continue with default freshness, since the server _did_ give
                // us a response after all.
                tracing::warn!("getting freshness lifetime: {e}");
                Self::default_freshness()
            });
        let etag = headers.get(ETAG).map(|header| header.as_bytes().to_owned());

        Self {
            fetched,
            freshness_lifetime,
            etag,
        }
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
                .collect::<std::collections::HashMap<_, _>>();

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

/// Implements the formatting specification from https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/If-Modified-Since
fn format_header_date(timestamp: Timestamp) -> crate::Result<HeaderValue> {
    let gmt = timestamp.in_tz("Etc/GMT")?;

    let weekday = match gmt.weekday() {
        jiff::civil::Weekday::Monday => "Mon",
        jiff::civil::Weekday::Tuesday => "Tue",
        jiff::civil::Weekday::Wednesday => "Wed",
        jiff::civil::Weekday::Thursday => "Thu",
        jiff::civil::Weekday::Friday => "Fri",
        jiff::civil::Weekday::Saturday => "Sat",
        jiff::civil::Weekday::Sunday => "Sun",
    };

    let day = gmt.day();

    let month = match gmt.month() {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => return Err(crate::Error::new("invalid month")),
    };

    let year = gmt.year();

    let hour = gmt.hour();
    let minute = gmt.minute();
    let second = gmt.second();

    let value =
        format!("{weekday}, {day:0>2} {month} {year:0>4} {hour:0>2}:{minute:0>2}:{second:0>2} GMT");
    Ok(HeaderValue::from_str(&value)?)
}
