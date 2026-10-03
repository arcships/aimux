//! A scripted [`Fetch`] transport and an environment-variable guard, for the
//! factory tests that replay recorded AI SDK requests.
//!
//! Included with `#[path = "common/mock_fetch.rs"] mod mock_fetch;`.

#![allow(dead_code)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;

use aimux_provider_utils::{Fetch, FetchError, FetchFunction, FetchRequest, FetchResponse};

/// One canned HTTP response.
#[derive(Clone)]
pub struct Canned {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Canned {
    pub fn json(body: &Value) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type".into(), "application/json".into())],
            body: serde_json::to_vec(body).unwrap(),
        }
    }
}

/// One request as the transport saw it.
#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    pub url: String,
    /// Lower-cased names (the `http` crate normalizes them).
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl Seen {
    pub fn json_body(&self) -> Value {
        serde_json::from_slice(&self.body).expect("request body is JSON")
    }
}

#[derive(Default)]
pub struct MockFetch {
    canned: Mutex<VecDeque<Canned>>,
    seen: Mutex<Vec<Seen>>,
}

impl MockFetch {
    pub fn new(canned: Vec<Canned>) -> Arc<Self> {
        Arc::new(Self {
            canned: Mutex::new(canned.into()),
            seen: Mutex::default(),
        })
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    pub fn transport(self: &Arc<Self>) -> FetchFunction {
        self.clone()
    }
}

#[async_trait]
impl Fetch for MockFetch {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.seen.lock().unwrap().push(Seen {
            method: request.method.to_string(),
            url: request.url.to_string(),
            headers: request
                .headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_string(),
                        value.to_str().expect("ASCII header").to_string(),
                    )
                })
                .collect(),
            body: request.body.to_vec(),
        });
        let canned = self
            .canned
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| FetchError::Other("no canned response left".into()))?;
        let mut headers = HeaderMap::new();
        for (name, value) in &canned.headers {
            headers.insert(
                HeaderName::try_from(name.as_str()).unwrap(),
                HeaderValue::try_from(value.as_str()).unwrap(),
            );
        }
        Ok(FetchResponse::from_bytes(
            StatusCode::from_u16(canned.status).unwrap(),
            headers,
            request.url,
            Bytes::from(canned.body),
        ))
    }
}

/// A scripted [`Fetch`] that answers every request from a function of the
/// request, so a test can script "poll answers 503 forever" without counting
/// canned responses. Every request is recorded.
pub struct RouteFetch {
    handler: Box<dyn Fn(&Seen) -> Canned + Send + Sync>,
    seen: Mutex<Vec<Seen>>,
}

impl RouteFetch {
    pub fn new(handler: impl Fn(&Seen) -> Canned + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            handler: Box::new(handler),
            seen: Mutex::default(),
        })
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    /// How many recorded requests had this method and URL path.
    pub fn count(&self, method: &str, path: &str) -> usize {
        self.seen()
            .iter()
            .filter(|seen| {
                seen.method == method
                    && url::Url::parse(&seen.url).is_ok_and(|url| url.path() == path)
            })
            .count()
    }

    pub fn transport(self: &Arc<Self>) -> FetchFunction {
        self.clone()
    }
}

#[async_trait]
impl Fetch for RouteFetch {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        let seen = Seen {
            method: request.method.to_string(),
            url: request.url.to_string(),
            headers: request
                .headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_string(),
                        value.to_str().expect("ASCII header").to_string(),
                    )
                })
                .collect(),
            body: request.body.to_vec(),
        };
        let canned = (self.handler)(&seen);
        self.seen.lock().unwrap().push(seen);
        let mut headers = HeaderMap::new();
        for (name, value) in &canned.headers {
            headers.insert(
                HeaderName::try_from(name.as_str()).unwrap(),
                HeaderValue::try_from(value.as_str()).unwrap(),
            );
        }
        Ok(FetchResponse::from_bytes(
            StatusCode::from_u16(canned.status).unwrap(),
            headers,
            request.url,
            Bytes::from(canned.body),
        ))
    }
}

/// Sets (or removes) an environment variable and restores it on drop. Tests
/// that use it are `#[serial]`.
pub struct EnvVar {
    name: &'static str,
    saved: Option<String>,
}

impl EnvVar {
    pub fn set(name: &'static str, value: Option<&str>) -> Self {
        let saved = std::env::var(name).ok();
        unsafe {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        Self { name, saved }
    }
}

impl Drop for EnvVar {
    fn drop(&mut self) {
        unsafe {
            match &self.saved {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }
}
