//! Google Vertex AI image model — implements the `ImageModel` trait.
//!
//! Aligned with Vercel AI SDK `GoogleVertexImageModel`
//! (`reference/ai/packages/google-vertex/src/google-vertex-image-model.ts`).
//!
//! Two code paths:
//! - **Imagen** models (non-`gemini-*`): `POST {base_url}/models/{id}:predict`
//! - **Gemini** image models (`gemini-*`): `POST {base_url}/models/{id}:generateContent`

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::{Map, Value, json};

use aimux_core::error::AiMuxError;
use aimux_core::image_model::{
    ImageCallOptions, ImageFile, ImageFileData, ImageModel, ImageOutputs, ImageResponse,
    ImageResult, ImageUsage,
};
use aimux_core::shared::Warning;

use aimux_provider_utils::response::ErrorStructure;
use aimux_provider_utils::{HttpBody, HttpMethod, HttpRequest, RetryConfig, send};

use super::{VertexAuth, VertexConfig};

const GOOGLE_ERROR_STRUCTURE: ErrorStructure = ErrorStructure {
    message_path: &["error", "message"],
    type_path: &["error", "status"],
};

fn is_gemini_model(model_id: &str) -> bool {
    model_id.starts_with("gemini-")
}

/// A Google Vertex AI image generation model.
///
/// Does **not** hold an HTTP client — `http::send` uses the process-wide shared
/// `Client` internally (RFC-0009 §4.1).
pub struct VertexImageModel {
    model_id: String,
    config: VertexConfig,
}

impl VertexImageModel {
    pub fn new(model_id: String, config: VertexConfig) -> Self {
        Self { model_id, config }
    }

    fn build_headers(&self, extra: Option<&HashMap<String, String>>) -> Vec<(String, String)> {
        let mut h = vec![("Content-Type".into(), "application/json".into())];
        match &self.config.auth {
            VertexAuth::BearerToken(token) => {
                h.push(("Authorization".into(), format!("Bearer {}", token)));
            }
            VertexAuth::ApiKey(key) => {
                h.push(("x-goog-api-key".into(), key.clone()));
            }
        }
        if let Some(extra) = extra {
            for (k, v) in extra {
                h.push((k.clone(), v.clone()));
            }
        }
        h
    }

    fn predict_endpoint(&self) -> String {
        format!("{}/models/{}:predict", self.config.base_url, self.model_id)
    }
    fn generate_content_endpoint(&self) -> String {
        let mp = if self.model_id.contains('/') {
            self.model_id.clone()
        } else {
            format!("models/{}", self.model_id)
        };
        format!("{}/{}:generateContent", self.config.base_url, mp)
    }

    fn get_base64_data(file: &ImageFile) -> Result<String, AiMuxError> {
        match file {
            ImageFile::Url { .. } => Err(AiMuxError::InvalidArgument(
                "URL-based images are not supported for Google Vertex image editing.".into(),
            )),
            ImageFile::File { data, .. } => match data {
                ImageFileData::Base64(s) => Ok(s.clone()),
                ImageFileData::Binary(b) => Ok(base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    b,
                )),
            },
        }
    }

    async fn do_generate_imagen(
        &self,
        options: &ImageCallOptions,
    ) -> Result<ImageResult, AiMuxError> {
        let mut warnings = Vec::new();
        if options.size.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "size".into(),
                details: Some(
                    "This model does not support the `size` option. Use `aspectRatio` instead."
                        .into(),
                ),
            });
        }

        // Parse provider options (googleVertex or vertex)
        let gv_opts = options
            .provider_options
            .get("googleVertex")
            .or_else(|| options.provider_options.get("vertex"));
        let edit_opts = gv_opts.and_then(|o| o.get("edit"));
        let edit_mode = edit_opts
            .and_then(|e| e.get("mode"))
            .and_then(|v| v.as_str());
        let base_steps = edit_opts.and_then(|e| e.get("baseSteps"));
        let mask_mode = edit_opts
            .and_then(|e| e.get("maskMode"))
            .and_then(|v| v.as_str());
        let mask_dilation = edit_opts.and_then(|e| e.get("maskDilation"));

        // Build other options (excluding "edit")
        let mut other_options = Map::new();
        if let Some(obj) = gv_opts.and_then(|v| v.as_object()) {
            for (k, v) in obj {
                if k != "edit" {
                    other_options.insert(k.clone(), v.clone());
                }
            }
        }

        let is_edit_mode = options.files.as_ref().is_some_and(|f| !f.is_empty());

        let mut parameters = Map::new();
        parameters.insert("sampleCount".into(), json!(options.n));
        if let Some(ar) = options.aspect_ratio {
            parameters.insert("aspectRatio".into(), json!(ar.to_string()));
        }
        if let Some(seed) = options.seed {
            parameters.insert("seed".into(), json!(seed));
        }

        let mut reference_images: Vec<Value> = Vec::new();

        if is_edit_mode {
            if let Some(ref files) = options.files {
                for (i, file) in files.iter().enumerate() {
                    reference_images.push(json!({
                        "referenceType": "REFERENCE_TYPE_RAW",
                        "referenceId": i + 1,
                        "referenceImage": { "bytesBase64Encoded": Self::get_base64_data(file)? }
                    }));
                }
            }
            if let Some(ref mask) = options.mask {
                let mut mask_config = Map::new();
                mask_config.insert(
                    "maskMode".into(),
                    json!(mask_mode.unwrap_or("MASK_MODE_USER_PROVIDED")),
                );
                if let Some(d) = mask_dilation {
                    mask_config.insert("dilation".into(), d.clone());
                }
                let file_count = options.files.as_ref().map_or(0, |f| f.len());
                reference_images.push(json!({
                    "referenceType": "REFERENCE_TYPE_MASK",
                    "referenceId": file_count + 1,
                    "referenceImage": { "bytesBase64Encoded": Self::get_base64_data(mask)? },
                    "maskImageConfig": Value::Object(mask_config),
                }));
            }
            parameters.insert(
                "editMode".into(),
                json!(edit_mode.unwrap_or("EDIT_MODE_INPAINT_INSERTION")),
            );
            if let Some(bs) = base_steps {
                parameters.insert("editConfig".into(), json!({ "baseSteps": bs }));
            }
        }

        for (k, v) in &other_options {
            parameters.insert(k.clone(), v.clone());
        }

        let body = if is_edit_mode {
            json!({
                "instances": [{ "prompt": options.prompt, "referenceImages": reference_images }],
                "parameters": parameters,
            })
        } else {
            json!({ "instances": [{ "prompt": options.prompt }], "parameters": parameters })
        };

        let headers = self.build_headers(options.headers.as_ref());

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.predict_endpoint(),
                headers,
                body: HttpBody::Json(body),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            RetryConfig::default(),
            &GOOGLE_ERROR_STRUCTURE,
        )
        .await?;
        let rh = resp.headers;
        let rb: Value = serde_json::from_slice(&resp.body)?;

        let images: Vec<String> = rb
            .get("predictions")
            .and_then(|p| p.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| {
                        p.get("bytesBase64Encoded")
                            .and_then(|v| v.as_str())
                            .map(String::from)
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Build provider metadata with revisedPrompt
        let image_metas: Vec<Value> = rb
            .get("predictions")
            .and_then(|p| p.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|p| {
                        let mut e = Map::new();
                        if let Some(rp) = p.get("prompt").and_then(|v| v.as_str()) {
                            e.insert("revisedPrompt".into(), json!(rp));
                        }
                        Value::Object(e)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let payload = json!({ "images": image_metas });
        let mut metadata = HashMap::new();
        metadata.insert("googleVertex".into(), payload.clone());
        metadata.insert("vertex".into(), payload);

        Ok(ImageResult {
            images: ImageOutputs::Base64(images),
            warnings,
            provider_metadata: Some(metadata),
            response: ImageResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: Some(rh),
            },
            usage: None,
        })
    }

    async fn do_generate_gemini(
        &self,
        options: &ImageCallOptions,
    ) -> Result<ImageResult, AiMuxError> {
        let mut warnings = Vec::new();
        if options.mask.is_some() {
            return Err(AiMuxError::UnsupportedFunctionality(
                "Gemini image models do not support mask-based image editing.".into(),
            ));
        }
        if options.n > 1 {
            return Err(AiMuxError::UnsupportedFunctionality("Gemini image models do not support generating a set number of images per call. Use n=1 or omit the n parameter.".into()));
        }
        if options.size.is_some() {
            warnings.push(Warning::Unsupported {
                feature: "size".into(),
                details: Some(
                    "This model does not support the `size` option. Use `aspectRatio` instead."
                        .into(),
                ),
            });
        }

        let mut parts: Vec<Value> = Vec::new();
        if let Some(ref p) = options.prompt {
            parts.push(json!({ "text": p }));
        }
        if let Some(ref files) = options.files {
            for file in files {
                match file {
                    ImageFile::Url { url } => {
                        return Err(AiMuxError::UnsupportedFunctionality(format!(
                            "URL-based input images with media type \"image/*\" are not passed as inline bytes. URL: {url}"
                        )));
                    }
                    ImageFile::File { media_type, data } => {
                        let ds = match data {
                            ImageFileData::Base64(s) => s.clone(),
                            ImageFileData::Binary(b) => base64::Engine::encode(
                                &base64::engine::general_purpose::STANDARD,
                                b,
                            ),
                        };
                        parts.push(json!({ "inlineData": { "mimeType": media_type, "data": ds } }));
                    }
                }
            }
        }

        let mut gc = Map::new();
        gc.insert("responseModalities".into(), json!(["IMAGE"]));
        if let Some(ar) = options.aspect_ratio {
            gc.insert(
                "imageConfig".into(),
                json!({ "aspectRatio": ar.to_string() }),
            );
        }
        if let Some(seed) = options.seed {
            gc.insert("seed".into(), json!(seed));
        }

        // Passthrough provider options
        let gv_opts = options
            .provider_options
            .get("googleVertex")
            .or_else(|| options.provider_options.get("vertex"));
        if let Some(obj) = gv_opts.and_then(|v| v.as_object()) {
            for (k, v) in obj {
                if matches!(k.as_str(), "responseModalities" | "imageConfig") {
                    continue;
                }
                gc.insert(k.clone(), v.clone());
            }
        }

        let body = json!({ "contents": [{ "role": "user", "parts": parts }], "generationConfig": Value::Object(gc) });
        let headers = self.build_headers(options.headers.as_ref());

        let resp = send(
            HttpRequest {
                method: HttpMethod::Post,
                url: self.generate_content_endpoint(),
                headers,
                body: HttpBody::Json(body),

                abort_signal: options.abort_signal.clone(),
                call_id: None,
                recording_context: None,
            },
            RetryConfig::default(),
            &GOOGLE_ERROR_STRUCTURE,
        )
        .await?;
        let rh = resp.headers;
        let rb: Value = serde_json::from_slice(&resp.body)?;

        let mut images: Vec<String> = Vec::new();
        if let Some(candidates) = rb.get("candidates").and_then(|c| c.as_array()) {
            for c in candidates {
                if let Some(parts) = c
                    .get("content")
                    .and_then(|c| c.get("parts"))
                    .and_then(|p| p.as_array())
                {
                    for p in parts {
                        if let Some(id) = p.get("inlineData")
                            && let Some(mt) = id.get("mimeType").and_then(|m| m.as_str())
                            && mt.starts_with("image/")
                            && let Some(d) = id.get("data").and_then(|d| d.as_str())
                        {
                            images.push(d.to_string());
                        }
                    }
                }
            }
        }

        let usage = rb.get("usageMetadata").map(|u| {
            let inp = u
                .get("promptTokenCount")
                .and_then(|v| v.as_u64())
                .map(|x| x as u32);
            let out = u
                .get("candidatesTokenCount")
                .and_then(|v| v.as_u64())
                .map(|x| x as u32);
            let tot = u
                .get("totalTokenCount")
                .and_then(|v| v.as_u64())
                .map(|x| x as u32);
            ImageUsage {
                input_tokens: inp,
                output_tokens: out,
                total_tokens: tot.or_else(|| Some(inp.unwrap_or(0) + out.unwrap_or(0))),
            }
        });

        let payload = json!({ "images": images.iter().map(|_| json!({})).collect::<Vec<_>>() });
        let mut metadata = HashMap::new();
        metadata.insert("googleVertex".into(), payload.clone());
        metadata.insert("vertex".into(), payload);

        Ok(ImageResult {
            images: ImageOutputs::Base64(images),
            warnings,
            provider_metadata: Some(metadata),
            response: ImageResponse {
                timestamp: Some(chrono::Utc::now().to_rfc3339()),
                model_id: Some(self.model_id.clone()),
                headers: Some(rh),
            },
            usage,
        })
    }
}

#[async_trait]
impl ImageModel for VertexImageModel {
    fn provider(&self) -> &str {
        "google.vertex"
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn max_images_per_call(&self) -> Option<u32> {
        if is_gemini_model(&self.model_id) {
            Some(10)
        } else {
            Some(4)
        }
    }

    async fn do_generate(&self, options: &ImageCallOptions) -> Result<ImageResult, AiMuxError> {
        if is_gemini_model(&self.model_id) {
            self.do_generate_gemini(options).await
        } else {
            self.do_generate_imagen(options).await
        }
    }
}
