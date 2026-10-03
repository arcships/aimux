//! A value that may be supplied directly, computed on demand, or awaited.
//!
//! The AI SDK's provider settings accept `string | (() => string | Promise<string>)`
//! for API keys and headers (`Resolvable<T>` in `@ai-sdk/provider-utils`).
//! This is the Rust shape of that union.

use std::fmt;
use std::sync::Arc;

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};

use aimux_core::AiMuxError;

/// A value of `T`, or a way to obtain one when a request is made.
///
/// `Future` and `AsyncFn` differ in *when* they compute: a `Future` is
/// evaluated at most once and every [`resolve`](Self::resolve) shares the
/// cached outcome (a promise value), while an `AsyncFn` / `Fn` runs again on
/// every resolve (a token refresher, a credential chain).
pub enum Resolvable<T> {
    /// A plain value.
    Value(T),
    /// A synchronous, fallible producer, called on every resolve.
    Fn(Arc<dyn Fn() -> Result<T, AiMuxError> + Send + Sync>),
    /// An asynchronous producer, called on every resolve.
    AsyncFn(Arc<dyn Fn() -> BoxFuture<'static, Result<T, AiMuxError>> + Send + Sync>),
    /// A one-shot future whose outcome (success or failure) is cached.
    Future(Shared<BoxFuture<'static, Result<T, AiMuxError>>>),
}

impl<T: Clone + Send + 'static> Resolvable<T> {
    /// Obtain the value.
    ///
    /// # Errors
    ///
    /// Returns whatever error the producer yields.
    pub async fn resolve(&self) -> Result<T, AiMuxError> {
        match self {
            Self::Value(value) => Ok(value.clone()),
            Self::Fn(produce) => produce(),
            Self::AsyncFn(produce) => produce().await,
            Self::Future(future) => future.clone().await,
        }
    }

    /// A synchronous producer evaluated on every resolve.
    #[must_use]
    pub fn from_fn(produce: impl Fn() -> Result<T, AiMuxError> + Send + Sync + 'static) -> Self {
        Self::Fn(Arc::new(produce))
    }

    /// An asynchronous producer evaluated on every resolve.
    #[must_use]
    pub fn from_async_fn<Fut>(produce: impl Fn() -> Fut + Send + Sync + 'static) -> Self
    where
        Fut: std::future::Future<Output = Result<T, AiMuxError>> + Send + 'static,
    {
        Self::AsyncFn(Arc::new(move || produce().boxed()))
    }

    /// A future evaluated at most once, its outcome shared by every resolve.
    #[must_use]
    pub fn from_future<Fut>(future: Fut) -> Self
    where
        Fut: std::future::Future<Output = Result<T, AiMuxError>> + Send + 'static,
    {
        Self::Future(future.boxed().shared())
    }
}

impl<T> From<T> for Resolvable<T> {
    fn from(value: T) -> Self {
        Self::Value(value)
    }
}

impl<T: Clone> Clone for Resolvable<T> {
    fn clone(&self) -> Self {
        match self {
            Self::Value(value) => Self::Value(value.clone()),
            Self::Fn(produce) => Self::Fn(produce.clone()),
            Self::AsyncFn(produce) => Self::AsyncFn(produce.clone()),
            Self::Future(future) => Self::Future(future.clone()),
        }
    }
}

impl<T> fmt::Debug for Resolvable<T> {
    /// Never prints the value: a `Resolvable` commonly carries credentials.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Value(_) => "Resolvable::Value(..)",
            Self::Fn(_) => "Resolvable::Fn(..)",
            Self::AsyncFn(_) => "Resolvable::AsyncFn(..)",
            Self::Future(_) => "Resolvable::Future(..)",
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[tokio::test]
    async fn value_and_fn_resolve() {
        assert_eq!(
            Resolvable::from("key".to_string()).resolve().await.unwrap(),
            "key"
        );
        let value = Resolvable::from_fn(|| Ok::<_, AiMuxError>(7_u32));
        assert_eq!(value.resolve().await.unwrap(), 7);
    }

    #[tokio::test]
    async fn async_fn_recomputes_on_every_resolve() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let value = Resolvable::from_async_fn(move || {
            let counter = counter.clone();
            async move { Ok::<_, AiMuxError>(counter.fetch_add(1, Ordering::SeqCst)) }
        });
        assert_eq!(value.resolve().await.unwrap(), 0);
        assert_eq!(value.resolve().await.unwrap(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn future_is_evaluated_once_and_shared_across_clones() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let value = Resolvable::from_future(async move {
            Ok::<_, AiMuxError>(counter.fetch_add(1, Ordering::SeqCst))
        });
        let clone = value.clone();
        assert_eq!(value.resolve().await.unwrap(), 0);
        assert_eq!(value.resolve().await.unwrap(), 0);
        assert_eq!(clone.resolve().await.unwrap(), 0);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn errors_propagate_and_a_future_caches_its_failure() {
        let value: Resolvable<String> =
            Resolvable::from_future(async { Err(AiMuxError::Other("boom".into())) });
        assert!(matches!(value.resolve().await, Err(AiMuxError::Other(_))));
        assert!(matches!(value.resolve().await, Err(AiMuxError::Other(_))));
    }

    #[test]
    fn debug_never_prints_the_value() {
        let value = Resolvable::from("sk-secret".to_string());
        assert!(!format!("{value:?}").contains("sk-secret"));
    }
}
