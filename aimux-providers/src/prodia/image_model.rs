//! Prodia image generation model.

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageOutputs, ImageResult};
use aimux_core::shared::Warning;

use crate::shared::{EndpointConfig, image_response, namespaced_options};

use super::provider::prodia_failed_response_handler;

/// The providerOptions key, read whatever the provider is named.
pub(crate) const NAMESPACE: &str = "prodia";

/// A Prodia image generation model.
pub struct ProdiaImageModel {
    model_id: String,
    config: EndpointConfig,
}

impl ProdiaImageModel {
    pub(crate) fn from_config(model_id: String, config: EndpointConfig) -> Self {
        Self { model_id, config }
    }
}

/// Parse a multipart body into parts (name, content_type, body).
fn parse_multipart(body: &[u8], boundary: &str) -> Vec<(String, String, Vec<u8>)> {
    let delimiter = format!("--{boundary}");
    let mut parts = Vec::new();
    let mut idx = 0;

    while let Some(start) = body[idx..]
        .windows(delimiter.len())
        .position(|w| w == delimiter.as_bytes())
    {
        let abs_start = idx + start + delimiter.len();
        if abs_start >= body.len() {
            break;
        }
        // Skip CRLF after boundary
        let content_start = if body[abs_start..].starts_with(b"\r\n") {
            abs_start + 2
        } else {
            abs_start
        };

        // Find next boundary
        let next_delimiter = format!("\r\n{delimiter}");
        let end = body[content_start..]
            .windows(next_delimiter.len())
            .position(|w| w == next_delimiter.as_bytes())
            .map(|p| content_start + p)
            .unwrap_or(body.len());

        let part = &body[content_start..end];

        // Parse headers
        let header_end = part
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .unwrap_or(part.len());
        let header_str = String::from_utf8_lossy(&part[..header_end]);
        let body_start = if header_end + 4 <= part.len() {
            header_end + 4
        } else {
            part.len()
        };

        let mut name = String::new();
        let mut content_type = String::new();
        for line in header_str.lines() {
            if line.to_lowercase().starts_with("content-disposition:")
                && let Some(n) = line.find("name=\"")
                && let Some(end) = line[n + 6..].find('"')
            {
                name = line[n + 6..n + 6 + end].to_string();
            }
            if line.to_lowercase().starts_with("content-type:") {
                content_type = line[13..].trim().to_string();
            }
        }

        parts.push((name, content_type, part[body_start..].to_vec()));
        idx = end;
    }

    parts
}

#[async_trait]
impl ImageModel for ProdiaImageModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_images_per_call(&self) -> Option<u32> {
        Some(1)
    }

    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        let warnings: Vec<Warning> = Vec::new();

        let prodia_opts = namespaced_options(Some(&options.provider_options), NAMESPACE);

        // Build job config
        let mut job_config = Map::new();
        if let Some(ref p) = options.prompt {
            job_config.insert("prompt".into(), json!(p));
        }

        // width/height
        if let Some(w) = prodia_opts.and_then(|o| o.get("width")) {
            job_config.insert("width".into(), w.clone());
        } else if let Some(s) = options.size {
            job_config.insert("width".into(), json!(s.width()));
        }
        if let Some(h) = prodia_opts.and_then(|o| o.get("height")) {
            job_config.insert("height".into(), h.clone());
        } else if let Some(s) = options.size {
            job_config.insert("height".into(), json!(s.height()));
        }

        if let Some(seed) = options.seed {
            job_config.insert("seed".into(), json!(seed));
        }
        if let Some(v) = prodia_opts.and_then(|o| o.get("steps")) {
            job_config.insert("steps".into(), v.clone());
        }
        if let Some(v) = prodia_opts.and_then(|o| o.get("stylePreset")) {
            job_config.insert("style_preset".into(), v.clone());
        }
        if let Some(v) = prodia_opts.and_then(|o| o.get("loras")) {
            job_config.insert("loras".into(), v.clone());
        }
        if let Some(v) = prodia_opts.and_then(|o| o.get("progressive")) {
            job_config.insert("progressive".into(), v.clone());
        }

        let body = json!({ "type": self.model_id, "config": job_config });

        let exchange = self.config.exchange(options.headers.as_ref()).await?;

        let resp = aimux_provider_utils::post_json_to_api(
            exchange
                .request(exchange.url("/job?price=true"), options)
                .generation_for(options),
            body,
            aimux_provider_utils::create_binary_response_handler(),
            prodia_failed_response_handler(),
        )
        .await?;

        let rh = resp.response_headers;
        let content_type = rh.get("content-type").cloned().unwrap_or_default();

        // Extract boundary
        let boundary = content_type
            .split(';')
            .find_map(|p| {
                let p = p.trim();
                p.strip_prefix("boundary=").map(|s| s.trim().to_string())
            })
            .ok_or_else(|| {
                AiMuxError::invalid_response_data(format!(
                    "Prodia response missing multipart boundary: {content_type}"
                ))
            })?;

        let body_bytes = resp.value.to_vec();

        // Parse multipart
        let parts = parse_multipart(&body_bytes, &boundary);

        let mut job_result: Option<Value> = None;
        let mut image_bytes: Option<Vec<u8>> = None;

        for (name, part_content_type, part_body) in parts {
            if name == "job" || name.contains("job") {
                let json_str = String::from_utf8_lossy(&part_body);
                job_result = serde_json::from_str(&json_str).ok();
            } else if name == "output"
                || name.contains("output")
                || part_content_type.starts_with("image/")
            {
                image_bytes = Some(part_body);
            }
        }

        let image_bytes = image_bytes.ok_or_else(|| {
            AiMuxError::invalid_response_data("Prodia multipart response missing output image")
        })?;
        let job_result = job_result.unwrap_or(Value::Null);

        // Build provider metadata
        let mut metadata = aimux_core::shared::NamespaceMap::new();
        let mut prodia_meta = Map::new();
        prodia_meta.insert("images".into(), json!([job_result]));
        metadata.insert(NAMESPACE.into(), prodia_meta);

        Ok(ImageResult {
            images: ImageOutputs::Binary(vec![image_bytes]),
            warnings,
            provider_metadata: Some(metadata),
            response: image_response(aimux_core::Timestamp::now(), &self.model_id, rh),
            usage: None,
        })
    }
}
