//! Cooperative caller cancellation for Core operations.

use tokio_util::sync::CancellationToken;

/// A cancellation signal analogous to the Web `AbortSignal`.
///
/// Timeouts are deliberately not encoded as cancellation reasons. Core owns
/// timeout deadlines and returns [`crate::AiMuxError::Timeout`] directly;
/// this type represents only caller-requested cancellation.
#[derive(Debug, Clone, Default)]
pub struct AbortSignal {
    token: CancellationToken,
}

impl AbortSignal {
    /// Create a fresh signal.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancel with the default reason.
    pub fn abort(&self) {
        self.token.cancel();
    }

    /// Whether cancellation has been requested.
    #[must_use]
    pub fn is_aborted(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Resolve when cancellation is requested.
    pub fn cancelled(&self) -> impl std::future::Future<Output = ()> + Send + 'static {
        self.token.clone().cancelled_owned()
    }
}
