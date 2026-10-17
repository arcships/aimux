//! Language-model middleware and wrapping, aligned with the upstream V4 hooks.

use std::sync::Arc;

use async_trait::async_trait;
use futures::future::BoxFuture;

use crate::error::AiMuxError;
use crate::language_model::{LanguageModel, SupportedUrls};
use crate::options::CallOptions;
use crate::result::{GenerateResult, StreamResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageModelOperation {
    Generate,
    Stream,
}

/// Optional hooks for transforming parameters and wrapping either operation.
/// The supplied callbacks invoke either operation using the transformed parameters.
#[async_trait]
pub trait LanguageModelMiddleware: Send + Sync {
    fn override_provider(&self, _model: &dyn LanguageModel) -> Option<String> {
        None
    }

    fn override_model_id(&self, _model: &dyn LanguageModel) -> Option<String> {
        None
    }

    /// Replaces the wrapped model's supported URLs. Async, as the AI SDK's
    /// `overrideSupportedUrls` may return a `PromiseLike` (provider@4.0.21
    /// `language-model-v4-middleware.ts:33-35`).
    async fn override_supported_urls(&self, _model: &dyn LanguageModel) -> Option<SupportedUrls> {
        None
    }

    async fn transform_params(
        &self,
        _operation: LanguageModelOperation,
        params: CallOptions,
        _model: &dyn LanguageModel,
    ) -> Result<CallOptions, AiMuxError> {
        Ok(params)
    }

    async fn wrap_generate(
        &self,
        _params: &CallOptions,
        _model: &dyn LanguageModel,
        do_generate: &(dyn Fn() -> BoxFuture<'_, Result<GenerateResult, AiMuxError>> + Send + Sync),
        _do_stream: &(dyn Fn() -> BoxFuture<'_, Result<StreamResult, AiMuxError>> + Send + Sync),
    ) -> Result<GenerateResult, AiMuxError> {
        do_generate().await
    }

    async fn wrap_stream(
        &self,
        _params: &CallOptions,
        _model: &dyn LanguageModel,
        _do_generate: &(
             dyn Fn() -> BoxFuture<'_, Result<GenerateResult, AiMuxError>> + Send + Sync
         ),
        do_stream: &(dyn Fn() -> BoxFuture<'_, Result<StreamResult, AiMuxError>> + Send + Sync),
    ) -> Result<StreamResult, AiMuxError> {
        do_stream().await
    }
}

struct WrappedLanguageModel {
    model: Arc<dyn LanguageModel>,
    middleware: Arc<dyn LanguageModelMiddleware>,
    provider: String,
    model_id: String,
}

/// Apply middleware in input order: the first entry is the outermost wrapper.
#[must_use]
pub fn wrap_language_model(
    model: Arc<dyn LanguageModel>,
    middleware: &[Arc<dyn LanguageModelMiddleware>],
    model_id: Option<&str>,
    provider_id: Option<&str>,
) -> Arc<dyn LanguageModel> {
    middleware.iter().rev().fold(model, |model, middleware| {
        let provider = provider_id
            .map(str::to_string)
            .or_else(|| middleware.override_provider(model.as_ref()))
            .unwrap_or_else(|| model.provider().to_string());
        let model_id = model_id
            .map(str::to_string)
            .or_else(|| middleware.override_model_id(model.as_ref()))
            .unwrap_or_else(|| model.model_id().to_string());
        Arc::new(WrappedLanguageModel {
            model,
            middleware: Arc::clone(middleware),
            provider,
            model_id,
        })
    })
}

#[async_trait]
impl LanguageModel for WrappedLanguageModel {
    fn provider(&self) -> &str {
        &self.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    /// The middleware's override, else the wrapped model's value, as
    /// `overrideSupportedUrls?.({ model }) ?? model.supportedUrls`
    /// (ai@7.0.127 `wrap-language-model.ts:79`), resolved when read.
    async fn supported_urls(&self) -> SupportedUrls {
        match self
            .middleware
            .override_supported_urls(self.model.as_ref())
            .await
        {
            Some(urls) => urls,
            None => self.model.supported_urls().await,
        }
    }

    async fn do_generate(&self, options: &CallOptions) -> Result<GenerateResult, AiMuxError> {
        let params = self
            .middleware
            .transform_params(
                LanguageModelOperation::Generate,
                options.clone(),
                self.model.as_ref(),
            )
            .await?;
        let do_generate = || self.model.do_generate(&params);
        let do_stream = || self.model.do_stream(&params);
        self.middleware
            .wrap_generate(&params, self.model.as_ref(), &do_generate, &do_stream)
            .await
    }

    async fn do_stream(&self, options: &CallOptions) -> Result<StreamResult, AiMuxError> {
        let params = self
            .middleware
            .transform_params(
                LanguageModelOperation::Stream,
                options.clone(),
                self.model.as_ref(),
            )
            .await?;
        let do_generate = || self.model.do_generate(&params);
        let do_stream = || self.model.do_stream(&params);
        self.middleware
            .wrap_stream(&params, self.model.as_ref(), &do_generate, &do_stream)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use regex::Regex;

    use super::*;

    fn urls(media_type: &str, pattern: &str) -> SupportedUrls {
        SupportedUrls(HashMap::from([(
            media_type.to_string(),
            vec![Regex::new(pattern).unwrap()],
        )]))
    }

    fn keys(urls: &SupportedUrls) -> Vec<&str> {
        let mut keys: Vec<&str> = urls.0.keys().map(String::as_str).collect();
        keys.sort_unstable();
        keys
    }

    struct Model;

    #[async_trait]
    impl LanguageModel for Model {
        fn provider(&self) -> &str {
            "mock"
        }

        fn model_id(&self) -> &str {
            "mock-model"
        }

        async fn supported_urls(&self) -> SupportedUrls {
            tokio::task::yield_now().await;
            urls("image/*", "^https://")
        }

        async fn do_generate(&self, _: &CallOptions) -> Result<GenerateResult, AiMuxError> {
            unreachable!("not called")
        }

        async fn do_stream(&self, _: &CallOptions) -> Result<StreamResult, AiMuxError> {
            unreachable!("not called")
        }
    }

    struct Override(Option<SupportedUrls>, AtomicUsize);

    #[async_trait]
    impl LanguageModelMiddleware for Override {
        async fn override_supported_urls(
            &self,
            model: &dyn LanguageModel,
        ) -> Option<SupportedUrls> {
            self.1.fetch_add(1, Ordering::SeqCst);
            assert_eq!(model.model_id(), "mock-model");
            tokio::task::yield_now().await;
            self.0.clone()
        }
    }

    /// `overrideSupportedUrls?.({ model }) ?? model.supportedUrls`
    /// (ai@7.0.127 `wrap-language-model.ts:79`): an override wins, its
    /// absence forwards the wrapped model's value, and both may be async.
    #[tokio::test]
    async fn supported_urls_come_from_the_override_else_the_model() {
        let overriding = Arc::new(Override(
            Some(urls("application/pdf", "^https://docs/")),
            AtomicUsize::new(0),
        ));
        let middleware: Arc<dyn LanguageModelMiddleware> = overriding.clone();
        let wrapped = wrap_language_model(Arc::new(Model), &[middleware], None, None);
        assert_eq!(keys(&wrapped.supported_urls().await), ["application/pdf"]);
        assert_eq!(overriding.1.load(Ordering::SeqCst), 1);

        let passing: Arc<dyn LanguageModelMiddleware> =
            Arc::new(Override(None, AtomicUsize::new(0)));
        let wrapped = wrap_language_model(Arc::new(Model), &[passing], None, None);
        assert_eq!(keys(&wrapped.supported_urls().await), ["image/*"]);

        // The first middleware is the outermost: its override wins.
        let inner: Arc<dyn LanguageModelMiddleware> =
            Arc::new(Override(Some(urls("audio/*", ".*")), AtomicUsize::new(0)));
        let wrapped = wrap_language_model(
            Arc::new(Model),
            &[
                overriding.clone() as Arc<dyn LanguageModelMiddleware>,
                inner,
            ],
            None,
            None,
        );
        assert_eq!(keys(&wrapped.supported_urls().await), ["application/pdf"]);
    }
}
