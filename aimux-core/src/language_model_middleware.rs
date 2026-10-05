//! Language-model middleware and wrapping, aligned with the upstream V4 hooks.

use std::sync::Arc;

use async_trait::async_trait;

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
/// The supplied model exposes both operations using the transformed parameters.
#[async_trait]
pub trait LanguageModelMiddleware: Send + Sync {
    fn override_provider(&self, _model: &dyn LanguageModel) -> Option<String> {
        None
    }

    fn override_model_id(&self, _model: &dyn LanguageModel) -> Option<String> {
        None
    }

    fn override_supported_urls(&self, _model: &dyn LanguageModel) -> Option<SupportedUrls> {
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
        params: &CallOptions,
        model: &dyn LanguageModel,
    ) -> Result<GenerateResult, AiMuxError> {
        model.do_generate(params).await
    }

    async fn wrap_stream(
        &self,
        params: &CallOptions,
        model: &dyn LanguageModel,
    ) -> Result<StreamResult, AiMuxError> {
        model.do_stream(params).await
    }
}

struct WrappedLanguageModel {
    model: Arc<dyn LanguageModel>,
    middleware: Arc<dyn LanguageModelMiddleware>,
    provider: String,
    model_id: String,
    supported_urls: SupportedUrls,
}

/// Apply middleware in input order: the first entry is the outermost wrapper.
#[must_use]
pub fn wrap_language_model(
    model: Arc<dyn LanguageModel>,
    middleware: &[Arc<dyn LanguageModelMiddleware>],
) -> Arc<dyn LanguageModel> {
    middleware.iter().rev().fold(model, |model, middleware| {
        let provider = middleware
            .override_provider(model.as_ref())
            .unwrap_or_else(|| model.provider().to_string());
        let model_id = middleware
            .override_model_id(model.as_ref())
            .unwrap_or_else(|| model.model_id().to_string());
        let supported_urls = middleware
            .override_supported_urls(model.as_ref())
            .unwrap_or_else(|| model.supported_urls());
        Arc::new(WrappedLanguageModel {
            model,
            middleware: Arc::clone(middleware),
            provider,
            model_id,
            supported_urls,
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

    fn supported_urls(&self) -> SupportedUrls {
        self.supported_urls.clone()
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
        self.middleware
            .wrap_generate(&params, self.model.as_ref())
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
        self.middleware
            .wrap_stream(&params, self.model.as_ref())
            .await
    }
}
