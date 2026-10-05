//! Image-model middleware and wrapping, aligned with the upstream V4 hooks.

use std::sync::Arc;

use async_trait::async_trait;
use futures::future::BoxFuture;

use crate::error::AiMuxError;
use crate::image_model::{ImageCallOptions, ImageModel, ImageResult};

/// Optional hooks for overriding model properties and wrapping generation.
#[async_trait]
pub trait ImageModelMiddleware: Send + Sync {
    fn override_provider(&self, _model: &dyn ImageModel) -> Option<String> {
        None
    }

    fn override_model_id(&self, _model: &dyn ImageModel) -> Option<String> {
        None
    }

    fn override_max_images_per_call(&self, _model: &dyn ImageModel) -> Option<u32> {
        None
    }

    /// `Some(None)` overrides support to unknown; `None` leaves it unchanged.
    fn override_supports_file_inputs(&self, _model: &dyn ImageModel) -> Option<Option<bool>> {
        None
    }

    /// `Some(None)` overrides support to unknown; `None` leaves it unchanged.
    fn override_supports_mask_inputs(&self, _model: &dyn ImageModel) -> Option<Option<bool>> {
        None
    }

    /// Transform the parameters passed to the wrapped operation.
    ///
    /// # Errors
    /// Returns an error if the parameters cannot be transformed.
    async fn transform_params(
        &self,
        params: ImageCallOptions,
        _model: &dyn ImageModel,
    ) -> Result<ImageCallOptions, AiMuxError> {
        Ok(params)
    }

    /// Wrap generation, with a callback using the transformed parameters.
    ///
    /// # Errors
    /// Returns a middleware error or the underlying generation error.
    async fn wrap_generate(
        &self,
        _params: &ImageCallOptions,
        _model: &dyn ImageModel,
        do_generate: &(dyn Fn() -> BoxFuture<'_, Result<ImageResult, AiMuxError>> + Send + Sync),
    ) -> Result<ImageResult, AiMuxError> {
        do_generate().await
    }
}

struct WrappedImageModel {
    model: Arc<dyn ImageModel>,
    middleware: Arc<dyn ImageModelMiddleware>,
    provider: String,
    model_id: String,
    max_images_per_call: Option<u32>,
    supports_file_inputs: Option<bool>,
    supports_mask_inputs: Option<bool>,
}

/// Apply middleware in input order: the first entry is the outermost wrapper.
#[must_use]
pub fn wrap_image_model(
    model: Arc<dyn ImageModel>,
    middleware: &[Arc<dyn ImageModelMiddleware>],
) -> Arc<dyn ImageModel> {
    middleware.iter().rev().fold(model, |model, middleware| {
        Arc::new(WrappedImageModel {
            provider: middleware
                .override_provider(model.as_ref())
                .unwrap_or_else(|| model.provider().to_string()),
            model_id: middleware
                .override_model_id(model.as_ref())
                .unwrap_or_else(|| model.model_id().to_string()),
            max_images_per_call: middleware
                .override_max_images_per_call(model.as_ref())
                .or_else(|| model.max_images_per_call()),
            supports_file_inputs: middleware
                .override_supports_file_inputs(model.as_ref())
                .unwrap_or_else(|| model.supports_file_inputs()),
            supports_mask_inputs: middleware
                .override_supports_mask_inputs(model.as_ref())
                .unwrap_or_else(|| model.supports_mask_inputs()),
            model,
            middleware: Arc::clone(middleware),
        })
    })
}

#[async_trait]
impl ImageModel for WrappedImageModel {
    fn provider(&self) -> &str {
        &self.provider
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn max_images_per_call(&self) -> Option<u32> {
        self.max_images_per_call
    }

    fn supports_file_inputs(&self) -> Option<bool> {
        self.supports_file_inputs
    }

    fn supports_mask_inputs(&self) -> Option<bool> {
        self.supports_mask_inputs
    }

    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        let params = self
            .middleware
            .transform_params(options.clone(), self.model.as_ref())
            .await?;
        let do_generate = || self.model.do_generate(&params);
        self.middleware
            .wrap_generate(&params, self.model.as_ref(), &do_generate)
            .await
    }
}
