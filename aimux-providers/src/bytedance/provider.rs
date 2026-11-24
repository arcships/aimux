//! Ark media provider factory. Chat remains available through the built-in preset.

use std::sync::OnceLock;

use aimux_core::AiMuxError;
use aimux_provider_utils::{AuthScheme, FetchFunction, HeaderMapOpt, Resolvable};

use crate::shared::{
    EndpointConfig, TransformRequestBody, VendorFactoryConfig, build_vendor_parts,
};

use super::{ByteDanceImageModel, ByteDanceVideoModel};

pub(crate) const DEFAULT_BASE_URL: &str = "https://ark.cn-beijing.volces.com/api/v3";
pub(crate) const API_KEY_ENV_VAR: &str = "ARK_API_KEY";

/// Settings for the Ark media factory; credentials are resolved per request.
#[derive(Clone, Default)]
pub struct ByteDanceProviderSettings {
    pub base_url: Option<String>,
    pub api_key: Option<Resolvable<String>>,
    pub headers: Option<HeaderMapOpt>,
    pub name: Option<String>,
    pub fetch: Option<FetchFunction>,
    pub transform_request_body: Option<TransformRequestBody>,
}

impl std::fmt::Debug for ByteDanceProviderSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::shared::debug_vendor_settings(
            f,
            "ByteDanceProviderSettings",
            &self.base_url,
            &self.api_key,
            &self.headers,
            &self.name,
            self.fetch.is_some(),
        )
    }
}

/// A native Seedance/Seedream factory. Model ids are opaque to support relay aliases.
pub struct ByteDanceProvider {
    image: EndpointConfig,
    video: EndpointConfig,
}

/// Create an Ark media provider with a custom relay URL when supplied.
///
/// # Errors
/// Returns an error if the explicit base URL is not an HTTP(S) URL with a host.
pub fn create_bytedance(
    settings: ByteDanceProviderSettings,
) -> Result<ByteDanceProvider, AiMuxError> {
    let parts = build_vendor_parts(
        settings.base_url,
        settings.api_key,
        settings.headers,
        settings.name,
        settings.fetch,
        VendorFactoryConfig {
            display: "ByteDance",
            default_base_url: DEFAULT_BASE_URL,
            api_key_env_var: API_KEY_ENV_VAR,
            default_name: "bytedance",
            auth: AuthScheme::Bearer,
            extra_headers: Vec::new(),
            package: None,
        },
    )?;
    Ok(ByteDanceProvider {
        image: parts
            .model_config("image")
            .with_transform_request_body(settings.transform_request_body.clone()),
        video: parts
            .model_config("video")
            .with_transform_request_body(settings.transform_request_body),
    })
}

/// Default Ark media provider. Construction does not read credentials.
pub fn bytedance() -> &'static ByteDanceProvider {
    static DEFAULT: OnceLock<ByteDanceProvider> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        create_bytedance(ByteDanceProviderSettings::default()).expect("valid default Ark URL")
    })
}

impl ByteDanceProvider {
    /// A Seedream image model, including endpoint ids and relay aliases.
    #[must_use]
    pub fn image(&self, model_id: &str) -> ByteDanceImageModel {
        ByteDanceImageModel::new(model_id.to_owned(), self.image.clone())
    }

    /// A Seedance video model, including endpoint ids and relay aliases.
    #[must_use]
    pub fn video(&self, model_id: &str) -> ByteDanceVideoModel {
        ByteDanceVideoModel::new(model_id.to_owned(), self.video.clone())
    }
}

crate::impl_modality_provider!(ByteDanceProvider {
    image_model: |p, id| p.image(id),
    video_model: |p, id| p.video(id),
});
