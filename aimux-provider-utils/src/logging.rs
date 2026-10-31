//! Unified logging (RFC-0014): connect the declared-but-unused `tracing` dep.
//!
//! aimux is a **library**, never a binary — so it never auto-initializes a
//! global subscriber on its own (that would fight consumers who bring their
//! own). Two explicit entry points exist:
//!
//! - **env auto-init** (`AIMUX_LOG` / `AIMUX_LOG_LEVEL`): lazily registers a
//!   `fmt` subscriber on first HTTP call **only if** no global subscriber is
//!   registered yet. Zero-friction for FFI/binding users: set an env var and
//!   logs appear.
//! - **programmatic API** (`init_logging`): idempotent (`Once`-guarded), used
//!   by Rust consumers and the shared `runtime.configure` operation.
//!
//! Privacy rules (§4.3): header *values*, URL query strings, and request/
//! response bodies are **never** logged by default. Bodies only appear when
//! `AIMUX_LOG_BODY=1` **and** the level is `trace`, truncated to 4KB with
//! auth-looking JSON fields redacted.

use std::io::Write;
use std::sync::{Once, OnceLock};

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;

/// Idempotency guard shared by environment, native API and ops initialization.
static INIT: Once = Once::new();
static REQUESTED_LEVEL: OnceLock<String> = OnceLock::new();

/// Default level: `warn` — only retries and failures are printed, normal
/// traffic stays silent.
const DEFAULT_LEVEL: &str = "warn";

/// Body log truncation limit (bytes), RFC-0014 §4.3.
const BODY_LOG_LIMIT: usize = 4096;

/// Initialize the global logger (idempotent). Every entry point funnels here.
///
/// Precedence: `AIMUX_LOG` (RUST_LOG-style directives) > `AIMUX_LOG_LEVEL`
/// (simple level name) > `level` argument > `DEFAULT_LEVEL`.
///
/// No-ops (without registering anything) when a global subscriber is already
/// installed by the consumer — we never override the host's own logger.
fn init_once(level: Option<&str>) -> bool {
    let mut applied = false;
    INIT.call_once(|| {
        if tracing::dispatcher::has_been_set() {
            return;
        }
        let filter = build_filter(level);
        if fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .try_init()
            .is_ok()
        {
            if let Some(level) = level {
                let _ = REQUESTED_LEVEL.set(level.to_owned());
            }
            applied = true;
        }
    });
    applied
}

/// Programmatic init: `aimux_providers::init_logging("debug")`.
///
/// Idempotent — later calls are no-ops. If the consumer already installed a
/// global subscriber, this does nothing.
pub fn init_logging(level: &str) {
    init_once(Some(level));
}

/// Whether aimux or the host has initialized the process logger.
#[must_use]
pub fn logging_initialized() -> bool {
    INIT.is_completed() || tracing::dispatcher::has_been_set()
}

/// Programmatic level requested at initialization; environment directives can override it.
#[must_use]
pub fn requested_logging_level() -> Option<&'static str> {
    REQUESTED_LEVEL.get().map(String::as_str)
}

/// Initialize logging through the shared guard, permitting identical requests.
/// # Errors
/// Returns an error when the host/environment or a different level froze logging.
pub fn configure_logging(level: &str) -> Result<(), LoggingFrozen> {
    if init_once(Some(level)) || requested_logging_level() == Some(level) {
        Ok(())
    } else {
        Err(LoggingFrozen)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("logging is frozen")]
pub struct LoggingFrozen;

/// Lazy env-driven auto-init, called by the shared API-call primitive.
///
/// Cheap when already initialized (`Once::is_completed` is a single atomic
/// load). Registers a subscriber only when an `AIMUX_LOG*` env var is present
/// **and** no global subscriber exists yet.
pub(crate) fn auto_init_from_env() {
    if INIT.is_completed() || tracing::dispatcher::has_been_set() {
        return;
    }
    let env_present =
        std::env::var_os("AIMUX_LOG").is_some() || std::env::var_os("AIMUX_LOG_LEVEL").is_some();
    if env_present {
        init_once(None);
    }
}

/// Whether request/response body trace logging is enabled (`AIMUX_LOG_BODY=1`).
#[must_use]
pub(crate) fn body_logging_enabled() -> bool {
    std::env::var("AIMUX_LOG_BODY")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// Build the [`EnvFilter`] honoring the precedence documented on
/// [`init_once`]. Unparseable directives fall back to `warn` rather than
/// panicking.
fn build_filter(level: Option<&str>) -> EnvFilter {
    if let Ok(directive) = std::env::var("AIMUX_LOG")
        && !directive.trim().is_empty()
    {
        return EnvFilter::try_new(directive).unwrap_or_else(|_| EnvFilter::new(DEFAULT_LEVEL));
    }
    let level = std::env::var("AIMUX_LOG_LEVEL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| level.map(str::to_owned))
        .unwrap_or_else(|| DEFAULT_LEVEL.to_owned());
    // "aimux" is the shared target prefix of every aimux crate
    // (aimux_core / aimux_providers / aimux_provider_utils / aimux_stream / aimux_ffi).
    EnvFilter::try_new(format!("aimux={level}")).unwrap_or_else(|_| EnvFilter::new(DEFAULT_LEVEL))
}

/// Truncate `body` to `BODY_LOG_LIMIT` bytes at a UTF-8 char boundary.
fn truncate(body: &str) -> &str {
    if body.len() <= BODY_LOG_LIMIT {
        return body;
    }
    let mut end = BODY_LOG_LIMIT;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    &body[..end]
}

/// Redact a body for trace logging: JSON object keys whose lowercased name
/// match the shared recording sensitivity policy have their values replaced
/// with `[REDACTED]`. Non-JSON bodies pass through unchanged. Always truncated
/// to `BODY_LOG_LIMIT`.
#[must_use]
pub(crate) fn redact_body(body: &str) -> String {
    let truncated = truncate(body);
    match serde_json::from_str::<serde_json::Value>(truncated) {
        Ok(value) => serde_json::to_string(&redact_error_context(value))
            .unwrap_or_else(|_| truncated.to_owned()),
        Err(_) => truncated.to_owned(),
    }
}

const ERROR_CONTEXT_MAX_DEPTH: usize = 8;
const ERROR_CONTEXT_MAX_ITEMS: usize = 128;
const ERROR_CONTEXT_MAX_STRING: usize = 4096;

/// Redact and bound structured values before exposing them in an API error.
#[must_use]
pub fn redact_error_context(value: serde_json::Value) -> serde_json::Value {
    redact_value(value, 0)
}

/// Serialize a redacted view of `value` directly to a JSON string without
/// cloning the input `serde_json::Value` tree or allocating an intermediate
/// redacted `Value` tree.
#[must_use]
pub(crate) fn redact_error_context_to_string(value: &serde_json::Value) -> Option<String> {
    struct RedactedRef<'a>(&'a serde_json::Value, usize);
    struct TruncatedItems(usize);

    impl serde::Serialize for TruncatedItems {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeMap;
            let mut m = serializer.serialize_map(Some(1))?;
            m.serialize_entry("__truncated_items__", &self.0)?;
            m.end()
        }
    }

    impl serde::Serialize for RedactedRef<'_> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::{SerializeMap, SerializeSeq};
            use serde_json::Value;
            if self.1 >= ERROR_CONTEXT_MAX_DEPTH {
                return serializer.serialize_str("[MAX_DEPTH]");
            }
            match self.0 {
                Value::Object(map) => {
                    let truncated = map.len() > ERROR_CONTEXT_MAX_ITEMS;
                    let len = map.len().min(ERROR_CONTEXT_MAX_ITEMS) + usize::from(truncated);
                    let mut out = serializer.serialize_map(Some(len))?;
                    for (index, (key, value)) in map.iter().enumerate() {
                        if index == ERROR_CONTEXT_MAX_ITEMS {
                            out.serialize_entry("__truncated__", &true)?;
                            break;
                        }
                        if aimux_core::recording::is_sensitive_key(key) {
                            out.serialize_entry(key, "[REDACTED]")?;
                        } else {
                            out.serialize_entry(key, &RedactedRef(value, self.1 + 1))?;
                        }
                    }
                    out.end()
                }
                Value::Array(items) => {
                    let original_len = items.len();
                    let truncated = original_len > ERROR_CONTEXT_MAX_ITEMS;
                    let len = original_len.min(ERROR_CONTEXT_MAX_ITEMS) + usize::from(truncated);
                    let mut seq = serializer.serialize_seq(Some(len))?;
                    for value in items.iter().take(ERROR_CONTEXT_MAX_ITEMS) {
                        seq.serialize_element(&RedactedRef(value, self.1 + 1))?;
                    }
                    if truncated {
                        seq.serialize_element(&TruncatedItems(
                            original_len - ERROR_CONTEXT_MAX_ITEMS,
                        ))?;
                    }
                    seq.end()
                }
                Value::String(value) if value.len() > ERROR_CONTEXT_MAX_STRING => {
                    serializer.serialize_str(&format!("[STRING {} bytes]", value.len()))
                }
                other => other.serialize(serializer),
            }
        }
    }

    serde_json::to_string(&RedactedRef(value, 0)).ok()
}

fn redact_value(value: serde_json::Value, depth: usize) -> serde_json::Value {
    use serde_json::Value;
    if depth >= ERROR_CONTEXT_MAX_DEPTH {
        return Value::String("[MAX_DEPTH]".to_owned());
    }
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len().min(ERROR_CONTEXT_MAX_ITEMS));
            for (index, (key, value)) in map.into_iter().enumerate() {
                if index == ERROR_CONTEXT_MAX_ITEMS {
                    out.insert("__truncated__".into(), Value::Bool(true));
                    break;
                }
                if aimux_core::recording::is_sensitive_key(&key) {
                    out.insert(key, Value::String("[REDACTED]".to_owned()));
                } else {
                    out.insert(key, redact_value(value, depth + 1));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => {
            let original_len = items.len();
            let mut items: Vec<_> = items
                .into_iter()
                .take(ERROR_CONTEXT_MAX_ITEMS)
                .map(|value| redact_value(value, depth + 1))
                .collect();
            if original_len > ERROR_CONTEXT_MAX_ITEMS {
                items.push(serde_json::json!({
                    "__truncated_items__": original_len - ERROR_CONTEXT_MAX_ITEMS
                }));
            }
            Value::Array(items)
        }
        Value::String(value) if value.len() > ERROR_CONTEXT_MAX_STRING => {
            Value::String(format!("[STRING {} bytes]", value.len()))
        }
        other => other,
    }
}

/// Sanitize request values before storing them in a public `ApiCallError`.
#[must_use]
pub(crate) fn redact_request_values(body: &crate::http::HttpBody) -> serde_json::Value {
    match body {
        crate::http::HttpBody::Json(value) => redact_error_context(value.clone()),
        crate::http::HttpBody::Bytes(bytes, content_type) => serde_json::json!({
            "content_type": content_type,
            "length": bytes.len(),
        }),
        crate::http::HttpBody::Empty => serde_json::json!({}),
    }
}

/// Small helper used by tests to capture formatted output.
#[doc(hidden)]
pub struct CaptureWriter(pub std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
    type Writer = CaptureSink;
    fn make_writer(&'a self) -> Self::Writer {
        CaptureSink(self.0.clone())
    }
}

#[doc(hidden)]
pub struct CaptureSink(pub std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl Write for CaptureSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// RAII guard: cleans up the logging env vars on drop (including panic),
    /// so a failed test cannot poison the next one.
    struct EnvGuard;

    impl EnvGuard {
        fn set(name: &str, value: &str) -> Self {
            // SAFETY: test process, env access is exclusive inside the serial group.
            unsafe { std::env::set_var(name, value) };
            Self
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: see set().
            unsafe {
                std::env::remove_var("AIMUX_LOG");
                std::env::remove_var("AIMUX_LOG_LEVEL");
                std::env::remove_var("AIMUX_LOG_BODY");
            }
        }
    }

    fn capture(
        filter: EnvFilter,
    ) -> (
        std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
        tracing::subscriber::DefaultGuard,
    ) {
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = fmt()
            .with_ansi(false)
            .with_env_filter(filter)
            .with_writer(CaptureWriter(captured.clone()))
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);
        (captured, guard)
    }

    fn captured_text(captured: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> String {
        String::from_utf8(captured.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn startup_logging_shares_native_environment_host_and_concurrent_initialization() {
        const CASE: &str = "AIMUX_LOGGING_TEST_CASE";
        let Ok(case) = std::env::var(CASE) else {
            for case in ["native", "environment", "host", "concurrent"] {
                let mut command = std::process::Command::new(std::env::current_exe().unwrap());
                command.args(["--exact", "logging::tests::startup_logging_shares_native_environment_host_and_concurrent_initialization", "--nocapture"])
                    .env(CASE, case).env_remove("AIMUX_LOG").env_remove("AIMUX_LOG_LEVEL");
                if case == "environment" {
                    command.env("AIMUX_LOG_LEVEL", "error");
                }
                assert!(command.status().unwrap().success(), "scenario {case}");
            }
            return;
        };
        assert!(!logging_initialized());
        match case.as_str() {
            "native" => {
                init_logging("info");
                assert!(configure_logging("info").is_ok());
                assert!(configure_logging("trace").is_err());
                assert_eq!(requested_logging_level(), Some("info"));
            }
            "environment" => {
                auto_init_from_env();
                assert!(logging_initialized());
                assert_eq!(requested_logging_level(), None);
                assert!(configure_logging("trace").is_err());
            }
            "host" => {
                tracing::subscriber::set_global_default(
                    tracing::subscriber::NoSubscriber::default(),
                )
                .unwrap();
                assert!(configure_logging("warn").is_err());
                assert_eq!(requested_logging_level(), None);
            }
            "concurrent" => {
                let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
                let callers: Vec<_> = (0..8)
                    .map(|index| {
                        let barrier = barrier.clone();
                        std::thread::spawn(move || {
                            let level = if index % 2 == 0 { "warn" } else { "trace" };
                            barrier.wait();
                            (level, configure_logging(level).is_ok())
                        })
                    })
                    .collect();
                for caller in callers {
                    let (level, applied) = caller.join().unwrap();
                    assert_eq!(applied, requested_logging_level() == Some(level));
                }
            }
            _ => panic!("unknown test scenario"),
        }
        assert!(logging_initialized());
    }

    #[test]
    fn redact_json_body() {
        let body = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"api_key":"sk-secret","nested":{"authorization":"Bearer xyz","token":"t"}}"#;
        let out = redact_body(body);
        assert!(!out.contains("sk-secret"), "api_key value leaked: {out}");
        assert!(!out.contains("Bearer xyz"), "authorization leaked: {out}");
        assert!(
            out.contains("\"api_key\":\"[REDACTED]\""),
            "api_key not redacted: {out}"
        );
        assert!(
            out.contains("\"authorization\":\"[REDACTED]\""),
            "authorization not redacted: {out}"
        );
        assert!(
            out.contains("\"model\":\"gpt-4o\""),
            "innocent field changed: {out}"
        );
    }

    #[test]
    fn redact_truncates_long_body() {
        let body = "x".repeat(BODY_LOG_LIMIT + 500);
        let out = redact_body(&body);
        assert!(out.len() <= BODY_LOG_LIMIT);
    }

    #[test]
    fn truncation_respects_utf8_char_boundaries() {
        // A 3-byte char straddling the limit must be dropped whole, not split
        // (a byte-index slice would panic mid-char).
        let body = format!("{}中中", "x".repeat(BODY_LOG_LIMIT - 1));
        let out = redact_body(&body);
        assert_eq!(out, "x".repeat(BODY_LOG_LIMIT - 1));
    }

    #[test]
    fn redact_non_json_passthrough() {
        let body = "not json at all";
        assert_eq!(redact_body(body), body);
    }

    #[test]
    #[serial]
    fn filter_precedence_env_level() {
        let _g = EnvGuard::set("AIMUX_LOG_LEVEL", "debug");
        let (captured, _guard) = capture(build_filter(None));
        tracing::debug!("visible-at-debug");
        let out = captured_text(&captured);
        assert!(
            out.contains("visible-at-debug"),
            "debug event not logged: {out}"
        );
    }

    #[test]
    #[serial]
    fn filter_precedence_aimux_log_wins() {
        let _g1 = EnvGuard::set("AIMUX_LOG", "aimux_provider_utils=error");
        let _g2 = EnvGuard::set("AIMUX_LOG_LEVEL", "debug");
        let (captured, _guard) = capture(build_filter(None));
        tracing::debug!("debug-must-be-filtered");
        tracing::error!("error-must-appear");
        let out = captured_text(&captured);
        assert!(
            !out.contains("debug-must-be-filtered"),
            "AIMUX_LOG=error should suppress debug: {out}"
        );
        assert!(
            out.contains("error-must-appear"),
            "error event missing: {out}"
        );
    }

    #[test]
    #[serial]
    fn filter_defaults_to_warn() {
        let (captured, _guard) = capture(build_filter(None));
        tracing::debug!("debug-must-be-suppressed");
        tracing::warn!("warn-must-appear");
        let out = captured_text(&captured);
        assert!(
            !out.contains("debug-must-be-suppressed"),
            "default level should suppress debug: {out}"
        );
        assert!(
            out.contains("warn-must-appear"),
            "warn event missing: {out}"
        );
    }

    #[test]
    #[serial]
    fn init_is_idempotent() {
        init_logging("debug");
        init_logging("trace"); // second call must be a no-op, not a panic
        assert!(tracing::dispatcher::has_been_set());
    }
}
