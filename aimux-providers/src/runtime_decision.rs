//! Official runtime decision endpoints. Connection identity stays with the runtime.
use std::collections::{BTreeMap, HashMap};

use aimux_core::AiMuxError;
use aimux_core::decision_model::*;
use aimux_core::recording::ProviderRecord;
use aimux_core::types::{TokenUsage, Usage};
use async_trait::async_trait;
use serde_json::{Map, Value, json};

use crate::decision_support::{self as support, invalid};
use crate::openai::OpenAIConfig;

pub(crate) fn supported(provider: &str) -> bool {
    matches!(
        provider,
        "ollama"
            | "llamacpp"
            | "localai"
            | "laya"
            | "vllm"
            | "sglang"
            | "cloudflare"
            | "cloudflare_workers_ai"
    )
}

pub(crate) struct RuntimeDecisionModel {
    pub(crate) config: OpenAIConfig,
    pub(crate) model_id: String,
}

impl RuntimeDecisionModel {
    pub(crate) fn new(model_id: &str, config: OpenAIConfig) -> Result<Self, AiMuxError> {
        if !supported(&config.provider) || config.body_overrides.is_some() {
            return Err(AiMuxError::UnsupportedFunctionality(
                "native decisions require an official adapter without generation body_overrides"
                    .into(),
            ));
        }
        if model_id.trim().is_empty() {
            return Err(AiMuxError::InvalidArgument(
                "decision model ID cannot be empty".into(),
            ));
        }
        Ok(Self {
            config,
            model_id: model_id.into(),
        })
    }

    fn systemone_request(
        &self,
        request: &DecisionCallOptions,
        mut options: Map<String, Value>,
    ) -> Result<(String, Value), AiMuxError> {
        let provider = self.provider();
        let mut body = crate::systemone::request_body(&self.model_id, request)?;
        if matches!(provider, "vllm" | "sglang") {
            for question in &request.questions {
                let valid = match question {
                    DecisionQuestion::Choice { options, .. } => {
                        options.len() <= if provider == "vllm" { 26 } else { 255 }
                    }
                    DecisionQuestion::Score { levels, .. } => {
                        levels.len() <= if provider == "vllm" { 26 } else { 10 }
                    }
                    _ => true,
                };
                if !valid || (provider == "vllm" && request.questions.len() > 64) {
                    return Err(AiMuxError::InvalidArgument(format!(
                        "{provider} SystemOne question or candidate limit exceeded"
                    )));
                }
            }
        }
        if provider == "laya" {
            let state_len = request.state.as_str().map_or_else(
                || request.state.to_string().chars().count(),
                |s| s.chars().count(),
            );
            let count: usize = request
                .questions
                .iter()
                .map(|q| match q {
                    DecisionQuestion::Boolean { .. } => 2,
                    DecisionQuestion::Choice { options, .. } => options.len(),
                    DecisionQuestion::Score { levels, .. } => levels.len(),
                })
                .sum();
            if state_len > 50_000 || count > 512 {
                return Err(AiMuxError::InvalidArgument(
                    "Laya state exceeds 50,000 characters or questions exceed 512 total options"
                        .into(),
                ));
            }
        }
        let allowed: &[&str] = match provider {
            "ollama" | "localai" => &["keep_alive"],
            "laya" => &[
                "max_len",
                "head_max_len",
                "task",
                "lang",
                "lang_guess",
                "min_confidence",
            ],
            "vllm" => &[
                "instructions",
                "samples",
                "auto_max",
                "auto_threshold",
                "steps",
                "think",
                "chunk_rows",
                "chunk_prompt",
                "sequential",
                "seed",
            ],
            "sglang" => &["chat_template_kwargs"],
            _ => &[],
        };
        if provider == "vllm" {
            if let Some(extensions) = options.remove("questions") {
                let extensions = extensions.as_object().ok_or_else(|| {
                    AiMuxError::InvalidArgument("vllm.questions must be an object".into())
                })?;
                for (id, extension) in extensions {
                    let target = body["questions"].get_mut(id).ok_or_else(|| {
                        AiMuxError::InvalidArgument(format!("unknown vllm question {id}"))
                    })?;
                    let extension = extension.as_object().ok_or_else(|| {
                        AiMuxError::InvalidArgument(
                            "vllm question extensions must be objects".into(),
                        )
                    })?;
                    for (key, value) in extension {
                        if !matches!(key.as_str(), "depends_on" | "ask_if" | "alone") {
                            return Err(AiMuxError::InvalidArgument(format!(
                                "unsupported vllm question option {key}"
                            )));
                        }
                        target[key] = value.clone();
                    }
                }
            }
            for q in &request.questions {
                if matches!(q, DecisionQuestion::Score { levels, .. } if levels.iter().any(|l| !matches!(l, DecisionDescription::Text(_))))
                {
                    return Err(AiMuxError::UnsupportedFunctionality("vLLM diffusion score levels must be text; its server stringifies structured levels".into()));
                }
            }
        }
        copy_options(&mut body, options, allowed)?;
        self.add_images(request, &mut body)?;
        let base = self.config.base_url.trim_end_matches('/');
        let url = if matches!(provider, "cloudflare" | "cloudflare_workers_ai") {
            let model = self
                .model_id
                .strip_prefix("@cf/cloudflare/")
                .unwrap_or(&self.model_id);
            if !matches!(model, "clef" | "clef-flash") {
                return Err(AiMuxError::UnsupportedFunctionality(
                    "Cloudflare native decisions support clef and clef-flash".into(),
                ));
            }
            for q in &request.questions {
                if q.id().len() > 100
                    || !q
                        .id()
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
                {
                    return Err(AiMuxError::InvalidArgument(
                        "Cloudflare question IDs require 1–100 ASCII letters, digits, _, . or -"
                            .into(),
                    ));
                }
            }
            body["model"] = json!(model);
            format!(
                "{}/run/@cf/cloudflare/{model}",
                base.strip_suffix("/v1").unwrap_or(base)
            )
        } else {
            format!("{base}/systemone")
        };
        let body_limit = match provider {
            "laya" => Some(2 * 1024 * 1024),
            "localai" if request.images.is_empty() => Some(64 * 1024),
            "localai" => Some(16 * 1024 * 1024),
            "cloudflare" | "cloudflare_workers_ai" => Some(13 * 1024 * 1024),
            _ => None,
        };
        if body_limit.is_some_and(|limit| body.to_string().len() > limit) {
            return Err(AiMuxError::InvalidArgument(format!(
                "{provider} decision request exceeds body limit"
            )));
        }
        Ok((url, body))
    }

    fn add_images(
        &self,
        request: &DecisionCallOptions,
        body: &mut Value,
    ) -> Result<(), AiMuxError> {
        if request.images.is_empty() {
            return Ok(());
        }
        let provider = self.provider();
        let mut images = Vec::new();
        let mut total = 0;
        let mut encoded_total = 0;
        for image in &request.images {
            let inline = support::inline_image(image)?;
            total += inline.decoded_len;
            encoded_total += inline.data_url.len();
            if matches!(provider, "localai" | "cloudflare" | "cloudflare_workers_ai") {
                let allowed = matches!(image.media_type.as_str(), "image/png" | "image/jpeg")
                    || (provider != "localai" && image.media_type == "image/webp");
                if !allowed || (provider != "localai" && inline.decoded_len > 4 * 1024 * 1024) {
                    return Err(AiMuxError::InvalidArgument(format!(
                        "{provider} unsupported image format or image exceeds 4 MiB"
                    )));
                }
            }
            if provider != "sglang" && image.detail.is_some() {
                return Err(AiMuxError::UnsupportedFunctionality(format!(
                    "{provider} does not support image detail"
                )));
            }
            images.push(match provider {
                "ollama" => json!(inline.base64),
                "sglang" => {
                    let mut value = json!({"url":inline.data_url});
                    if let Some(detail) = &image.detail {
                        value["detail"] = json!(detail);
                    }
                    value
                }
                _ => json!(inline.data_url),
            });
        }
        if matches!(provider, "localai" | "cloudflare" | "cloudflare_workers_ai")
            && total > 8 * 1024 * 1024
        {
            return Err(AiMuxError::InvalidArgument(format!(
                "{provider} images exceed 8 MiB decoded"
            )));
        }
        if provider == "localai" && encoded_total > 12 * 1024 * 1024 {
            return Err(AiMuxError::InvalidArgument(
                "LocalAI image URLs exceed 12 MiB".into(),
            ));
        }
        body["images"] = json!(images);
        Ok(())
    }

    fn systemone_response(
        &self,
        request: &DecisionCallOptions,
        raw: Value,
        headers: HashMap<String, String>,
    ) -> Result<DecisionResult, AiMuxError> {
        let mut data = if matches!(self.provider(), "cloudflare" | "cloudflare_workers_ai") {
            if raw.get("success") != Some(&json!(true)) {
                return Err(invalid("Cloudflare decision request failed"));
            }
            raw.get("result")
                .cloned()
                .ok_or_else(|| invalid("Cloudflare omitted result"))?
        } else {
            raw.clone()
        };
        let mut skipped = Vec::new();
        if self.provider() == "vllm" {
            let diagnostics = data.get("diagnostics").cloned().unwrap_or(Value::Null);
            if let Some(answers) = data.get_mut("answers").and_then(Value::as_object_mut) {
                for (id, answer) in answers.iter() {
                    if answer.is_null()
                        && diagnostics.get("skipped").and_then(|d| d.get(id)).is_some()
                    {
                        skipped.push(id.clone());
                    }
                }
                for id in &skipped {
                    answers.remove(id);
                }
            }
        }
        let mut result = crate::systemone::convert_response(
            serde_json::from_value(data)
                .map_err(|e| invalid(format!("{} SystemOne: {e}", self.provider())))?,
            raw.clone(),
            headers,
            crate::systemone::ResponseProfile {
                provider: self.provider(),
                rounding: DecisionRounding::default(),
                probability_source: DecisionProbabilitySource::Native,
            },
        )?;
        for id in skipped {
            result.answers.insert(id, DecisionAnswer::Skipped);
        }
        if self.provider() == "laya" {
            for (id, answer) in &mut result.answers {
                if raw["answers"][id]["abstention"] == "abstained" {
                    *answer = DecisionAnswer::Abstention;
                }
            }
        }
        // Cloudflare's usage belongs to the nested result; keep the full envelope as raw.
        if let Some(usage) = &mut result.usage {
            usage.raw = if raw.get("result").is_some() {
                raw["result"].get("usage").cloned()
            } else {
                raw.get("usage").cloned()
            };
        }
        result.latency_ms = raw.get("latency_ms").and_then(Value::as_f64);
        result.validate(request)?;
        Ok(result)
    }

    fn sglang_request(
        &self,
        request: &DecisionCallOptions,
        options: Map<String, Value>,
    ) -> Result<Value, AiMuxError> {
        let mut questions = Vec::new();
        for question in &request.questions {
            if matches!(question, DecisionQuestion::Score { levels, .. } if levels.len() > 10) {
                return Err(AiMuxError::InvalidArgument(
                    "SGLang decisions scores support at most 10 levels".into(),
                ));
            }
            let wire = match question {
                DecisionQuestion::Boolean {
                    id,
                    instructions,
                    criteria,
                } => {
                    let mut wire = json!({"id":id,"type":"yes_no","question":instructions});
                    if let Some(criteria) = criteria {
                        if let Some(yes) = &criteria.true_description {
                            wire["yes"] = json!(yes);
                        }
                        if let Some(no) = &criteria.false_description {
                            wire["no"] = json!(no);
                        }
                    }
                    wire
                }
                DecisionQuestion::Choice {
                    id,
                    instructions,
                    options,
                } => {
                    if !(2..=26).contains(&options.len()) {
                        return Err(AiMuxError::InvalidArgument(
                            "SGLang decisions choices require 2–26 options".into(),
                        ));
                    }
                    let options: Vec<_> = options
                        .iter()
                        .map(|o| {
                            let mut v = json!({"name":o.label});
                            if let Some(d) = &o.description {
                                v["description"] = json!(d);
                            }
                            v
                        })
                        .collect();
                    json!({"id":id,"type":"choice","question":instructions,"options":options})
                }
                DecisionQuestion::Score {
                    id,
                    instructions,
                    levels,
                } => json!({"id":id,"type":"score","question":instructions,"levels":levels}),
            };
            questions.push(wire);
        }
        let mut body = json!({"model":self.model_id,"input":request.state,"questions":questions});
        copy_options(
            &mut body,
            options,
            &[
                "temperature",
                "chat_template_kwargs",
                "prompt_format_version",
                "return_prompt_token_ids",
            ],
        )?;
        self.add_images(request, &mut body)?;
        Ok(body)
    }
}

pub(crate) fn copy_options(
    body: &mut Value,
    options: Map<String, Value>,
    allowed: &[&str],
) -> Result<(), AiMuxError> {
    for (key, value) in options {
        if !allowed.contains(&key.as_str()) {
            return Err(AiMuxError::InvalidArgument(format!(
                "unsupported decision option {key}"
            )));
        }
        body[key] = value;
    }
    Ok(())
}

pub(crate) fn result_from_distributions(
    request: &DecisionCallOptions,
    distributions: Vec<Vec<f64>>,
    model: String,
    provider: &str,
    raw: Value,
    headers: HashMap<String, String>,
) -> Result<DecisionResult, AiMuxError> {
    if model.trim().is_empty() || distributions.len() != request.questions.len() {
        return Err(invalid(
            "decision scoring response has wrong model or row count",
        ));
    }
    let mut answers = BTreeMap::new();
    for (question, probabilities) in request.questions.iter().zip(distributions) {
        let answer = match question {
            DecisionQuestion::Boolean { .. } => {
                if probabilities.len() != 2
                    || probabilities
                        .iter()
                        .any(|p| !p.is_finite() || !(0.0..=1.0).contains(p))
                    || (probabilities.iter().sum::<f64>() - 1.0).abs()
                        > f64::from(f32::EPSILON) * 2.0
                {
                    return Err(invalid(
                        "Boolean scoring requires a normalized yes/no distribution",
                    ));
                }
                DecisionAnswer::Boolean {
                    probability_true: probabilities[0],
                }
            }
            DecisionQuestion::Choice { options, .. } => {
                if probabilities.len() != options.len() {
                    return Err(invalid("choice scoring row has wrong number of candidates"));
                }
                let best = probabilities
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.total_cmp(b.1).then_with(|| b.0.cmp(&a.0)))
                    .ok_or_else(|| invalid("empty distribution"))?
                    .0;
                DecisionAnswer::Choice {
                    selected: options[best].label.clone(),
                    value: None,
                    confidence: None,
                    probabilities: Some(
                        options
                            .iter()
                            .map(|o| o.label.clone())
                            .zip(probabilities)
                            .collect(),
                    ),
                }
            }
            DecisionQuestion::Score { levels, .. } => DecisionAnswer::Score {
                expected_value: probabilities
                    .iter()
                    .enumerate()
                    .map(|(i, p)| i as f64 * p)
                    .sum(),
                levels: levels.clone(),
                probabilities: Some(probabilities),
                confidence: None,
            },
        };
        answers.insert(question.id().into(), answer);
    }
    let usage = raw
        .get("usage")
        .filter(|v| !v.is_null())
        .map(|value| Usage {
            input_tokens: TokenUsage {
                total: value
                    .get("prompt_tokens")
                    .and_then(Value::as_u64)
                    .and_then(|n| n.try_into().ok()),
                ..Default::default()
            },
            output_tokens: TokenUsage {
                total: value
                    .get("completion_tokens")
                    .and_then(Value::as_u64)
                    .and_then(|n| n.try_into().ok()),
                ..Default::default()
            },
            raw: Some(value.clone()),
        });
    let result = DecisionResult {
        rounding: DecisionRounding::default(),
        answers,
        provider: provider.into(),
        model,
        model_version: None,
        probability_source: DecisionProbabilitySource::LogitScoring,
        usage,
        latency_ms: None,
        provider_metadata: Some(HashMap::from([(provider.into(), raw.clone())])),
        response: Some(DecisionResponse {
            headers: Some(headers),
            body: Some(raw),
        }),
    };
    result.validate(request)?;
    Ok(result)
}

fn sglang_response(
    request: &DecisionCallOptions,
    raw: Value,
    headers: HashMap<String, String>,
) -> Result<DecisionResult, AiMuxError> {
    let answers = raw["answers"]
        .as_object()
        .ok_or_else(|| invalid("missing SGLang answers"))?;
    if answers.len() != request.questions.len() {
        return Err(invalid("SGLang answer IDs do not match request"));
    }
    let mut distributions = Vec::new();
    for question in &request.questions {
        let answer = answers
            .get(question.id())
            .ok_or_else(|| invalid("missing SGLang answer"))?;
        let (kind, labels) = match question {
            DecisionQuestion::Boolean { .. } => {
                ("yes_no", vec!["yes".to_string(), "no".to_string()])
            }
            DecisionQuestion::Choice { options, .. } => {
                ("choice", options.iter().map(|o| o.label.clone()).collect())
            }
            DecisionQuestion::Score { levels, .. } => {
                ("score", (0..levels.len()).map(|i| i.to_string()).collect())
            }
        };
        if answer["type"] != kind {
            return Err(invalid("wrong SGLang answer type"));
        }
        let p = answer["probabilities"]
            .as_object()
            .ok_or_else(|| invalid("missing SGLang probabilities"))?;
        if p.len() != labels.len() {
            return Err(invalid("wrong SGLang probability count"));
        }
        distributions.push(
            labels
                .iter()
                .map(|label| {
                    p.get(label)
                        .and_then(Value::as_f64)
                        .ok_or_else(|| invalid("missing SGLang candidate probability"))
                })
                .collect::<Result<Vec<_>, _>>()?,
        );
    }
    let mut result = result_from_distributions(
        request,
        distributions,
        raw["model"].as_str().unwrap_or_default().into(),
        "sglang",
        raw.clone(),
        headers,
    )?;
    for (id, answer) in &mut result.answers {
        // Keep server selection/expectation, and validate against the same distribution.
        match answer {
            DecisionAnswer::Choice { selected, .. } => {
                *selected = raw["answers"][id]["choice"]
                    .as_str()
                    .ok_or_else(|| invalid("missing SGLang choice"))?
                    .into()
            }
            DecisionAnswer::Score { expected_value, .. } => {
                *expected_value = raw["answers"][id]["score"]
                    .as_f64()
                    .ok_or_else(|| invalid("missing SGLang score"))?
            }
            _ => {}
        }
    }
    result.validate(request)?;
    Ok(result)
}

#[async_trait]
impl DecisionModel for RuntimeDecisionModel {
    fn provider(&self) -> &str {
        &self.config.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn retry_config(&self) -> aimux_core::retry::RetryConfig {
        self.config.retry_config
    }
    fn capabilities(&self) -> DecisionCapabilities {
        let provider = self.provider();
        DecisionCapabilities {
            supports_images: provider != "laya",
            max_images: match provider {
                "localai" => Some(8),
                "cloudflare" | "cloudflare_workers_ai" => Some(4),
                _ => None,
            },
            supports_typed_choices: false,
            rounding: DecisionRounding::default(),
            probability_source: DecisionProbabilitySource::Native,
            supports_boolean: true,
            supports_choice: true,
            supports_score: true,
            returns_distributions: true,
            max_questions: match provider {
                "ollama" | "localai" | "laya" | "cloudflare" | "cloudflare_workers_ai" => Some(64),
                _ => None,
            },
            min_choices: Some(if provider == "sglang" { 1 } else { 2 }),
            max_choices: match provider {
                "laya" => Some(100),
                "cloudflare" | "cloudflare_workers_ai" => Some(255),
                _ => None,
            },
            max_score_levels: match provider {
                "laya" => Some(32),
                "llamacpp" | "cloudflare" | "cloudflare_workers_ai" => Some(10),
                _ => None,
            },
        }
    }
    fn config_snapshot(&self) -> ProviderRecord {
        let mut record = crate::openai::config_snapshot_from_config(
            self.provider(),
            self.model_id(),
            &self.config,
        );
        record.profile = Some(
            json!({"decision_default_protocol":"systemone", "decision_capabilities":self.capabilities()}),
        );
        record
    }
    async fn do_decide(&self, request: &DecisionCallOptions) -> Result<DecisionResult, AiMuxError> {
        let mut options = support::options(request, self.provider())?;
        let protocol = options.remove("protocol").unwrap_or(json!("systemone"));
        match (self.provider(), protocol.as_str()) {
            ("sglang", Some("decisions")) => {
                let body = self.sglang_request(request, options)?;
                let url = format!("{}/decisions", self.config.base_url.trim_end_matches('/'));
                support::post(&self.config, request, &url, body, |raw, headers| {
                    sglang_response(request, raw, headers)
                })
                .await
            }
            ("sglang", Some("score")) | ("vllm", Some("generative_scoring")) => {
                crate::decision_scoring::score(self, request, options).await
            }
            (_, Some("systemone")) => {
                let (url, body) = self.systemone_request(request, options)?;
                support::post(&self.config, request, &url, body, |raw, headers| {
                    self.systemone_response(request, raw, headers)
                })
                .await
            }
            _ => Err(AiMuxError::UnsupportedFunctionality(format!(
                "{} does not support decision protocol {protocol}",
                self.provider()
            ))),
        }
    }
}
