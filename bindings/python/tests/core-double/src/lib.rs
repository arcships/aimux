//! Test-only error surface; not used for real-core integration builds.
//! abort_signal.rs is copied verbatim from aimux-core by the test runner.
mod abort_signal;
pub use abort_signal::AbortSignal;

#[derive(Debug)]
pub enum AiMuxError {
    Aborted(String),
}

impl std::fmt::Display for AiMuxError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Aborted(message) => formatter.write_str(message),
        }
    }
}

impl serde::Serialize for AiMuxError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Aborted(message) => serde::Serialize::serialize(
                &serde_json::json!({"name":"AbortError", "message":message}),
                serializer,
            ),
        }
    }
}
