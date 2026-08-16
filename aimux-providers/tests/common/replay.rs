//! Cassette replay infrastructure for offline provider tests.
//!
//! Loads JSON cassette files (see RFC 0003) describing recorded HTTP exchanges
//! and mounts them onto a [`wiremock`] `MockServer`. A provider under test is
//! pointed at the server's URI; when it makes a request, the matching cassette's
//! recorded response is returned verbatim — no network, no API keys.
//!
//! ## Cassette format
//!
//! ```json
//! {
//!   "source": "rig (MIT)",
//!   "provider": "anthropic",
//!   "scenario": "completion_smoke",
//!   "request": {
//!     "path": "/v1/messages",
//!     "method": "POST",
//!     "headers": { "content-type": "application/json" },
//!     "body": { "model": "claude-3-haiku-20240307", "messages": [ ... ] }
//!   },
//!   "response": {
//!     "status": 200,
//!     "headers": { "content-type": "application/json" },
//!     "body": "{\"id\":\"msg_...\", ...}"
//!   }
//! }
//! ```
//!
//! `request.body` may be a JSON object or a string. `response.body` is always a
//! raw string — non-streaming responses are JSON text, streaming responses are
//! the original SSE text. The body bytes are returned to the caller untouched.
//!
//! ## Matching
//!
//! Cassettes are grouped by `(method, path)`; one `Mock` is registered per
//! group with a dynamic [`Respond`] that scores every cassette in the group
//! against the live request body and returns the best match:
//!
//! 1. **Path + method** select the group (the mock's matcher).
//! 2. **Feature scoring** within the group: each stable top-level *scalar*
//!    field of a cassette's recorded `request.body` that equals the
//!    corresponding field of the incoming request body adds to that cassette's
//!    score. `model` is weighted highest, then `stream`, then any other scalar
//!    (strings/bools/numbers). Non-scalar fields (`messages`, `tools`, ...) are
//!    ignored because they carry timestamps/IDs and are not stable.
//! 3. **Tie-break**: the highest-scoring cassette wins; ties resolve to file
//!    order (files are sorted by name), so a group with no discriminative
//!    fields falls back to the first cassette — exactly the RFC's "return the
//!    first matching cassette" rule.
//!
//! ## Usage
//!
//! ```no_run
//! # async fn example() {
//! use wiremock::MockServer;
//! use aimux_providers::anthropic::AnthropicConfig;
//! use reqwest::Client;
//!
//! let server = MockServer::start().await;
//! common::replay::mount_cassettes(&server, "tests/cassettes/anthropic").await;
//!
//! let config = AnthropicConfig::new("test-key").with_base_url(server.uri());
//! // ... build the model, call do_generate / do_stream, assert on the parsed
//! // result. The provider sees the cassette's real recorded response.
//! # }
//! ```
//!
//! [`Respond`]: wiremock::Respond

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

// ─── public API ─────────────────────────────────────────────────────────────

/// Load every `*.json` cassette in `dir` (non-recursive) and mount them on
/// `server`. Returns the number of cassettes mounted.
///
/// Cassettes are grouped by `(method, path)`; one `Mock` is registered per
/// group with a dynamic responder that selects the best-matching cassette per
/// request (see the [module docs](self#matching) for the matching rules).
///
/// Panics if `dir` cannot be read, or if any file fails to parse as a cassette
/// — test infrastructure fails fast on malformed fixtures.
pub async fn mount_cassettes(server: &MockServer, dir: impl AsRef<Path>) -> usize {
    let cassettes = load_cassettes(dir.as_ref());
    mount_entries(server, cassettes).await
}

/// Start a fresh [`MockServer`], mount all cassettes in `dir`, and return both
/// the server and the cassette count. Convenience wrapper around
/// [`MockServer::start`] + [`mount_cassettes`].
#[allow(dead_code)]
pub async fn start(dir: impl AsRef<Path>) -> (MockServer, usize) {
    let server = MockServer::start().await;
    let n = mount_cassettes(&server, dir).await;
    (server, n)
}

// ─── internals ──────────────────────────────────────────────────────────────

/// A single recorded exchange, pre-processed for mounting.
struct Cassette {
    method: String,
    req_path: String,
    /// Recorded `request.body` — used for feature scoring. May be an object,
    /// a string, or null.
    req_body: Value,
    /// The pre-built response template (status + recorded headers + raw body).
    response: ResponseTemplate,
}

/// A dynamic [`Respond`] that holds every cassette sharing one `(method, path)`
/// and picks the best match for each incoming request.
struct CassetteRespond {
    entries: Vec<Cassette>,
}

impl Respond for CassetteRespond {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        // Parse the incoming body once; non-JSON bodies simply score 0 across
        // the board and fall back to the first cassette.
        let req_body: Option<Value> = serde_json::from_slice(&request.body).ok();
        let req_obj = req_body.as_ref().and_then(|v| v.as_object());

        // Pick the highest-scoring cassette; ties keep the first (file order).
        // `entries` is non-empty — a group is only created when ≥1 cassette
        // shares its (method, path).
        let mut best_idx = 0usize;
        let mut best_score = score(&self.entries[0].req_body, req_obj);
        for (i, c) in self.entries.iter().enumerate().skip(1) {
            let s = score(&c.req_body, req_obj);
            if s > best_score {
                best_score = s;
                best_idx = i;
            }
        }
        self.entries[best_idx].response.clone()
    }
}

/// Score a cassette against the incoming request body. See the module docs for
/// the weighting rationale.
fn score(cassette_body: &Value, req_obj: Option<&serde_json::Map<String, Value>>) -> u32 {
    let cas_obj = match cassette_body.as_object() {
        Some(o) => o,
        None => return 0,
    };
    let req_obj = match req_obj {
        Some(o) => o,
        None => return 0,
    };

    let mut total = 0u32;
    for (key, cas_val) in cas_obj {
        // Only compare stable scalar fields; arrays/objects (messages, tools,
        // response_format, ...) are skipped — they carry unstable data.
        if !is_scalar(cas_val) {
            continue;
        }
        if let Some(req_val) = req_obj.get(key)
            && req_val == cas_val
        {
            total += match key.as_str() {
                "model" => 10,
                "stream" => 5,
                _ => 1,
            };
        }
    }
    total
}

fn is_scalar(v: &Value) -> bool {
    matches!(v, Value::Bool(_) | Value::Number(_) | Value::String(_))
}

/// Percent-decode a path component. Only the path is decoded (not query
/// strings), and only the percent-encoding that rig's cassettes carry —
/// `:` (`%3A`), `/` (`%2F`), space (`%20`), etc. Decoding is lenient: an
/// invalid escape sequence is left as-is rather than erroring.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_val(bytes[i + 1]);
            let lo = hex_val(bytes[i + 2]);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Read and parse all cassettes in `dir`, sorted by filename for deterministic
/// fallback ordering. Panics with the offending file path on any error.
fn load_cassettes(dir: &Path) -> Vec<Cassette> {
    let entries = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => panic!("replay: cannot read cassette dir {}: {}", dir.display(), e),
    };

    let mut files: Vec<_> = entries
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .collect();
    files.sort();

    let mut out = Vec::with_capacity(files.len());
    for f in files {
        let label = f.display().to_string();

        let text = match fs::read_to_string(&f) {
            Ok(t) => t,
            Err(e) => panic!("replay: cannot read {label}: {e}"),
        };
        let v: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => panic!("replay: invalid JSON in {label}: {e}"),
        };

        let req = v
            .get("request")
            .unwrap_or_else(|| panic!("replay: {label} missing `request` object"));
        let resp = v
            .get("response")
            .unwrap_or_else(|| panic!("replay: {label} missing `response` object"));

        let method = req
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or_else(|| panic!("replay: {label} missing `request.method`"))
            .to_ascii_uppercase();
        let req_path = req
            .get("path")
            .and_then(|p| p.as_str())
            .unwrap_or_else(|| panic!("replay: {label} missing `request.path`"))
            // Percent-decode the recorded path so it matches the path the HTTP
            // client actually sends. Cassettes sourced from rig record paths
            // verbatim from the wire, where characters like `:` in Bedrock model
            // IDs arrive percent-encoded (`amazon.nova-lite-v1%3A0`). reqwest
            // sends these characters unencoded, so without decoding the
            // wiremock `path()` matcher misses and every request 404s.
            .to_string();
        let req_path = percent_decode(&req_path);
        let status = resp
            .get("status")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_else(|| panic!("replay: {label} missing numeric `response.status`"))
            as u16;
        let req_body = req.get("body").cloned().unwrap_or(Value::Null);

        let body_str = match resp.get("body") {
            None => "",
            Some(Value::String(s)) => s.as_str(),
            Some(other) => {
                panic!("replay: {label} `response.body` must be a raw string, got {other}")
            }
        };

        // `set_body_bytes` sets the body without forcing a content-type, so the
        // recorded headers (including content-type) are returned verbatim.
        let mut template = ResponseTemplate::new(status);
        if let Some(headers) = resp.get("headers").and_then(|h| h.as_object()) {
            for (name, val) in headers {
                if let Some(s) = val.as_str() {
                    // append (not insert) so multi-valued headers such as
                    // Set-Cookie survive if a cassette ever carries them.
                    template = template.append_header(name.as_str(), s);
                }
            }
        }
        template = template.set_body_bytes(body_str.as_bytes().to_vec());

        out.push(Cassette {
            method,
            req_path,
            req_body,
            response: template,
        });
    }
    out
}

/// Group cassettes by `(method, path)` and register one `Mock` per group.
async fn mount_entries(server: &MockServer, cassettes: Vec<Cassette>) -> usize {
    let count = cassettes.len();

    let mut groups: BTreeMap<(String, String), Vec<Cassette>> = BTreeMap::new();
    for c in cassettes {
        groups
            .entry((c.method.clone(), c.req_path.clone()))
            .or_default()
            .push(c);
    }

    for ((m, p), entries) in groups {
        Mock::given(method(m))
            .and(path(p))
            .respond_with(CassetteRespond { entries })
            .mount(server)
            .await;
    }

    count
}

#[cfg(test)]
mod tests {
    //! Unit tests for the pure scoring logic (no HTTP). The end-to-end replay
    //! behaviour is exercised by `tests/replay_test.rs`.

    use super::*;
    use serde_json::json;

    #[test]
    fn score_is_zero_when_either_body_is_not_an_object() {
        // No incoming body to score against.
        assert_eq!(score(&json!({"model": "x"}), None), 0);
        // Cassette body not an object.
        let req = json!({"model": "x"});
        assert_eq!(
            score(&json!("not an object"), Some(req.as_object().unwrap())),
            0
        );
        assert_eq!(score(&Value::Null, Some(req.as_object().unwrap())), 0);
    }

    #[test]
    fn score_weights_model_above_stream_above_others() {
        let req = json!({
            "model": "claude-3-haiku-20240307",
            "stream": true,
            "max_tokens": 64,
            "messages": [{ "role": "user", "content": "Hi" }]
        });
        let req_obj = req.as_object().unwrap();

        // model + stream + max_tokens all match → 10 + 5 + 1 = 16
        let exact = json!({
            "model": "claude-3-haiku-20240307",
            "stream": true,
            "max_tokens": 64
        });
        assert_eq!(score(&exact, Some(req_obj)), 16);

        // model matches, stream differs → 10
        let wrong_stream = json!({
            "model": "claude-3-haiku-20240307",
            "stream": false
        });
        assert_eq!(score(&wrong_stream, Some(req_obj)), 10);

        // nothing matches → 0
        let wrong_model = json!({ "model": "claude-3-opus-20240229" });
        assert_eq!(score(&wrong_model, Some(req_obj)), 0);
    }

    #[test]
    fn score_ignores_non_scalar_fields() {
        let req = json!({ "model": "x", "messages": [{ "role": "user" }] });
        let req_obj = req.as_object().unwrap();
        // `messages` is an array — ignored even if it differs.
        let cas = json!({ "model": "x", "messages": [{ "role": "system" }] });
        assert_eq!(score(&cas, Some(req_obj)), 10);
    }
}
