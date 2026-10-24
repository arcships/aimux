//! Official runtime decision endpoints. Connection identity stays with the runtime.
use std::collections::HashMap;

use aimux_core::AiMuxError;
use aimux_core::ErrorCause;
use aimux_core::decision_model::*;
use async_trait::async_trait;
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use crate::decision_support::{
    self as support, DecisionConfig, DecisionImage, DecisionProbabilitySource, candidate_count,
    invalid, provider_metadata, response_info,
};
use crate::openai_compatible::config::CompatModelConfig;

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

fn is_cloudflare(provider: &str) -> bool {
    matches!(provider, "cloudflare" | "cloudflare_workers_ai")
}

/// What a runtime's decision endpoint accepts, checked before any request.
struct Limits {
    images: bool,
    max_images: Option<usize>,
    max_questions: Option<usize>,
    min_choices: usize,
    max_choices: Option<usize>,
    max_score_levels: Option<usize>,
}

fn limits(provider: &str) -> Limits {
    let cloudflare = is_cloudflare(provider);
    Limits {
        images: provider != "laya",
        max_images: match provider {
            "localai" => Some(8),
            _ if cloudflare => Some(4),
            _ => None,
        },
        max_questions: match provider {
            "ollama" | "localai" | "laya" => Some(64),
            _ if cloudflare => Some(64),
            _ => None,
        },
        min_choices: if provider == "sglang" { 1 } else { 2 },
        max_choices: match provider {
            "laya" => Some(100),
            _ if cloudflare => Some(255),
            _ => None,
        },
        max_score_levels: match provider {
            "laya" => Some(32),
            "llamacpp" => Some(10),
            _ if cloudflare => Some(10),
            _ => None,
        },
    }
}

/// Decisions of an official runtime, made from its OpenAI-compatible preset.
pub(crate) struct RuntimeDecisionModel {
    /// The preset name; it selects the runtime's protocol and limits.
    pub(crate) provider: String,
    pub(crate) config: DecisionConfig,
    pub(crate) model_id: String,
}

impl RuntimeDecisionModel {
    pub(crate) fn new(
        provider: &str,
        model_id: &str,
        config: CompatModelConfig,
    ) -> Result<Self, AiMuxError> {
        if !supported(provider) {
            return Err(AiMuxError::UnsupportedFunctionality(format!(
                "provider '{provider}' does not provide decision models"
            )));
        }
        if model_id.trim().is_empty() {
            return Err(AiMuxError::invalid_argument(
                "decision model ID cannot be empty",
            ));
        }
        Ok(Self {
            provider: provider.into(),
            config: DecisionConfig::Compatible(config),
            model_id: model_id.into(),
        })
    }

    /// Reject what the runtime cannot take before any request.
    fn preflight(
        &self,
        request: &DecisionCallOptions,
        images: &[DecisionImage],
    ) -> Result<(), AiMuxError> {
        let provider = self.provider();
        let limits = limits(provider);
        if !images.is_empty() && !limits.images {
            return Err(AiMuxError::UnsupportedFunctionality(format!(
                "{provider} decisions do not accept images"
            )));
        }
        if let Some(max) = limits.max_images.filter(|max| images.len() > *max) {
            return Err(AiMuxError::invalid_argument(format!(
                "{provider} accepts at most {max} decision images"
            )));
        }
        if let Some(max) = limits
            .max_questions
            .filter(|max| request.questions.len() > *max)
        {
            return Err(AiMuxError::invalid_argument(format!(
                "{provider} accepts at most {max} decision questions"
            )));
        }
        for (id, question) in &request.questions {
            let within = match question {
                DecisionQuestion::Choice { criteria, .. } => {
                    criteria.len() >= limits.min_choices
                        && limits.max_choices.is_none_or(|max| criteria.len() <= max)
                }
                DecisionQuestion::Score { criteria, .. } => limits
                    .max_score_levels
                    .is_none_or(|max| criteria.len() <= max),
                DecisionQuestion::Boolean { .. } => true,
            };
            if !within {
                return Err(AiMuxError::invalid_argument(format!(
                    "question {id:?} exceeds {provider} criteria limits"
                )));
            }
        }
        Ok(())
    }

    fn systemone_request(
        &self,
        request: &DecisionCallOptions,
        mut options: Map<String, Value>,
        images: &[DecisionImage],
    ) -> Result<(String, Value), AiMuxError> {
        let provider = self.provider();
        let mut body = crate::systemone::request_body(&self.model_id, request);
        if matches!(provider, "vllm" | "sglang") {
            for question in request.questions.values() {
                let valid = match question {
                    DecisionQuestion::Choice { criteria, .. } => {
                        criteria.len() <= if provider == "vllm" { 26 } else { 255 }
                    }
                    DecisionQuestion::Score { criteria, .. } => {
                        criteria.len() <= if provider == "vllm" { 26 } else { 10 }
                    }
                    DecisionQuestion::Boolean { .. } => true,
                };
                if !valid || (provider == "vllm" && request.questions.len() > 64) {
                    return Err(AiMuxError::invalid_argument(format!(
                        "{provider} SystemOne question or candidate limit exceeded"
                    )));
                }
            }
        }
        if provider == "laya" {
            let state_len = match &request.state {
                DecisionInput::Text(text) => text.chars().count(),
                state => serde_json::to_string(state)?.chars().count(),
            };
            let count: usize = request.questions.values().map(candidate_count).sum();
            if state_len > 50_000 || count > 512 {
                return Err(AiMuxError::invalid_argument(
                    "Laya state exceeds 50,000 characters or questions exceed 512 total options",
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
                    AiMuxError::invalid_argument("vllm.questions must be an object")
                })?;
                for (id, extension) in extensions {
                    let target = body["questions"].get_mut(id).ok_or_else(|| {
                        AiMuxError::invalid_argument(format!("unknown vllm question {id}"))
                    })?;
                    let extension = extension.as_object().ok_or_else(|| {
                        AiMuxError::invalid_argument("vllm question extensions must be objects")
                    })?;
                    for (key, value) in extension {
                        if !matches!(key.as_str(), "depends_on" | "ask_if" | "alone") {
                            return Err(AiMuxError::invalid_argument(format!(
                                "unsupported vllm question option {key}"
                            )));
                        }
                        target[key] = value.clone();
                    }
                }
            }
            let structured_level =
                |level: &Option<DecisionInput>| !matches!(level, Some(DecisionInput::Text(_)));
            if request.questions.values().any(|question| {
                matches!(question, DecisionQuestion::Score { criteria, .. } if criteria.iter().any(structured_level))
            }) {
                return Err(AiMuxError::UnsupportedFunctionality(
                    "vLLM diffusion score levels must be text; its server stringifies structured levels"
                        .into(),
                ));
            }
        }
        copy_options(&mut body, options, allowed)?;
        self.add_images(images, &mut body)?;
        let base = self.config.base_url()?;
        let base = base.as_str();
        let url = if is_cloudflare(provider) {
            let model = self
                .model_id
                .strip_prefix("@cf/cloudflare/")
                .unwrap_or(&self.model_id);
            if !matches!(model, "clef" | "clef-flash") {
                return Err(AiMuxError::UnsupportedFunctionality(
                    "Cloudflare native decisions support clef and clef-flash".into(),
                ));
            }
            for id in request.questions.keys() {
                if id.is_empty()
                    || id.len() > 100
                    || !id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
                {
                    return Err(AiMuxError::invalid_argument(
                        "Cloudflare question IDs require 1–100 ASCII letters, digits, _, . or -",
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
            "localai" if images.is_empty() => Some(64 * 1024),
            "localai" => Some(16 * 1024 * 1024),
            _ if is_cloudflare(provider) => Some(13 * 1024 * 1024),
            _ => None,
        };
        if body_limit.is_some_and(|limit| body.to_string().len() > limit) {
            return Err(AiMuxError::invalid_argument(format!(
                "{provider} decision request exceeds body limit"
            )));
        }
        Ok((url, body))
    }

    fn add_images(&self, images: &[DecisionImage], body: &mut Value) -> Result<(), AiMuxError> {
        if images.is_empty() {
            return Ok(());
        }
        let provider = self.provider();
        let restricted = provider == "localai" || is_cloudflare(provider);
        let mut wire = Vec::new();
        let mut total = 0;
        let mut encoded_total = 0;
        for image in images {
            let inline = support::inline_image(image)?;
            total += inline.decoded_len;
            encoded_total += inline.data_url.len();
            if restricted {
                let allowed = matches!(image.media_type.as_str(), "image/png" | "image/jpeg")
                    || (provider != "localai" && image.media_type == "image/webp");
                if !allowed || (provider != "localai" && inline.decoded_len > 4 * 1024 * 1024) {
                    return Err(AiMuxError::invalid_argument(format!(
                        "{provider} unsupported image format or image exceeds 4 MiB"
                    )));
                }
            }
            if provider != "sglang" && image.detail.is_some() {
                return Err(AiMuxError::UnsupportedFunctionality(format!(
                    "{provider} does not support image detail"
                )));
            }
            wire.push(match provider {
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
        if restricted && total > 8 * 1024 * 1024 {
            return Err(AiMuxError::invalid_argument(format!(
                "{provider} images exceed 8 MiB decoded"
            )));
        }
        if provider == "localai" && encoded_total > 12 * 1024 * 1024 {
            return Err(AiMuxError::invalid_argument(
                "LocalAI image URLs exceed 12 MiB",
            ));
        }
        body["images"] = json!(wire);
        Ok(())
    }

    fn systemone_response(
        &self,
        request: &DecisionCallOptions,
        raw: Value,
        headers: HashMap<String, String>,
    ) -> Result<DecisionResult, AiMuxError> {
        let provider = self.provider();
        let data = if is_cloudflare(provider) {
            if raw.get("success") != Some(&json!(true)) {
                return Err(invalid("Cloudflare decision request failed"));
            }
            raw.get("result")
                .cloned()
                .ok_or_else(|| invalid("Cloudflare omitted result"))?
        } else {
            raw.clone()
        };
        // A decision answers every question: a question vLLM skipped under
        // `ask_if` fails the call instead of becoming an answer.
        if provider == "vllm" {
            let skipped = data["answers"].as_object().and_then(|answers| {
                answers.iter().find_map(|(id, answer)| {
                    let why = data["diagnostics"]["skipped"].get(id)?;
                    answer.is_null().then_some((id, why))
                })
            });
            if let Some((id, why)) = skipped {
                return Err(invalid(format!(
                    "vllm skipped question {id:?} ({why}); a decision must answer every question"
                )));
            }
        }
        // SGLang's generation-model path reports full-vocabulary label mass;
        // its dedicated decision checkpoint readout explicitly omits it.
        let probability_source = if provider == "sglang"
            && data["answers"].as_object().is_some_and(|answers| {
                answers
                    .values()
                    .any(|answer| answer.get("x_label_mass").is_some())
            }) {
            DecisionProbabilitySource::LogitScoring
        } else {
            DecisionProbabilitySource::Native
        };
        // Laya keeps its prediction when it abstains; the abstention is
        // provider metadata of that answer.
        let abstentions: Vec<(String, Value)> = data["answers"]
            .as_object()
            .map(|answers| {
                answers
                    .iter()
                    .filter_map(|(id, answer)| {
                        let abstention = answer.get("abstention").filter(|v| !v.is_null())?;
                        Some((id.clone(), abstention.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let latency = raw.get("latency_ms").filter(|v| v.is_number()).cloned();
        let mut result = crate::systemone::convert_response(
            request,
            serde_json::from_value(data).map_err(|e| AiMuxError::InvalidResponseData {
                message: format!("{provider} SystemOne: {e}"),
                cause: Some(ErrorCause::source(&e)),
            })?,
            raw,
            headers,
            crate::systemone::ResponseProfile {
                provider,
                rounding: None,
                probability_source,
            },
        )?;
        let metadata = result
            .provider_metadata
            .get_or_insert_with(Default::default)
            .entry(provider.to_owned())
            .or_default();
        if provider == "laya" && !abstentions.is_empty() {
            let answers = metadata
                .entry("answers")
                .or_insert_with(|| Value::Object(Map::new()));
            for (id, abstention) in abstentions {
                answers[&id]["abstention"] = abstention;
            }
        }
        if let Some(latency) = latency {
            metadata.insert("latencyMs".into(), latency);
        }
        Ok(result)
    }

    fn sglang_request(
        &self,
        request: &DecisionCallOptions,
        options: Map<String, Value>,
        images: &[DecisionImage],
    ) -> Result<Value, AiMuxError> {
        let mut questions = Vec::new();
        for (id, question) in &request.questions {
            let wire = match question {
                DecisionQuestion::Boolean {
                    instructions,
                    criteria,
                } => {
                    let mut wire = json!({"id":id,"type":"yes_no","question":instructions});
                    if let Some(criteria) = criteria {
                        if let Some(Some(yes)) = &criteria.true_ {
                            wire["yes"] = json!(yes);
                        }
                        if let Some(Some(no)) = &criteria.false_ {
                            wire["no"] = json!(no);
                        }
                    }
                    wire
                }
                DecisionQuestion::Choice {
                    instructions,
                    criteria,
                } => {
                    if !(2..=26).contains(&criteria.len()) {
                        return Err(AiMuxError::invalid_argument(
                            "SGLang decisions choices require 2–26 options",
                        ));
                    }
                    let options: Vec<_> = criteria
                        .iter()
                        .map(|(label, description)| {
                            let mut option = json!({"name":label});
                            if let Some(description) = description {
                                option["description"] = json!(description);
                            }
                            option
                        })
                        .collect();
                    json!({"id":id,"type":"choice","question":instructions,"options":options})
                }
                DecisionQuestion::Score {
                    instructions,
                    criteria,
                } => {
                    if criteria.len() > 10 {
                        return Err(AiMuxError::invalid_argument(
                            "SGLang decisions scores support at most 10 levels",
                        ));
                    }
                    json!({"id":id,"type":"score","question":instructions,"levels":criteria})
                }
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
        self.add_images(images, &mut body)?;
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
            return Err(AiMuxError::invalid_argument(format!(
                "unsupported decision option {key}"
            )));
        }
        body[key] = value;
    }
    Ok(())
}

/// V4 answers from one candidate distribution per question, in request
/// order: P(yes) for Boolean, the most likely option for Choice, the mean
/// level for Score.
pub(crate) fn result_from_distributions(
    request: &DecisionCallOptions,
    distributions: Vec<Vec<f64>>,
    model: &str,
    provider: &str,
    raw: Value,
    headers: HashMap<String, String>,
) -> Result<DecisionResult, AiMuxError> {
    if model.trim().is_empty() || distributions.len() != request.questions.len() {
        return Err(invalid(
            "decision scoring response has wrong model or row count",
        ));
    }
    let mut answers = IndexMap::new();
    for ((id, question), probabilities) in request.questions.iter().zip(distributions) {
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
                    probability: probabilities[0],
                }
            }
            DecisionQuestion::Choice { criteria, .. } => {
                if probabilities.len() != criteria.len() {
                    return Err(invalid("choice scoring row has wrong number of candidates"));
                }
                let best = probabilities
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.total_cmp(b.1).then_with(|| b.0.cmp(&a.0)))
                    .ok_or_else(|| invalid("empty distribution"))?
                    .0;
                DecisionAnswer::Choice {
                    choice: criteria
                        .get_index(best)
                        .map(|(label, _)| label.clone())
                        .unwrap_or_default(),
                    probabilities: Some(criteria.keys().cloned().zip(probabilities).collect()),
                }
            }
            DecisionQuestion::Score { .. } => DecisionAnswer::Score {
                score: probabilities
                    .iter()
                    .enumerate()
                    .map(|(i, p)| i as f64 * p)
                    .sum(),
                probabilities: Some(
                    probabilities
                        .into_iter()
                        .enumerate()
                        .map(|(i, p)| (i.to_string(), p))
                        .collect(),
                ),
            },
        };
        answers.insert(id.clone(), answer);
    }
    let tokens = |key: &str| {
        raw["usage"]
            .get(key)
            .and_then(Value::as_u64)
            .and_then(|n| n.try_into().ok())
    };
    let usage = raw
        .get("usage")
        .filter(|usage| !usage.is_null())
        .map(|_| DecisionUsage {
            input_tokens: tokens("prompt_tokens"),
            output_tokens: tokens("completion_tokens"),
        });
    validate_decision_answers(&request.questions, &answers, None)?;
    Ok(DecisionResult {
        answers,
        rounding: None,
        usage,
        warnings: Vec::new(),
        provider_metadata: Some(provider_metadata(
            provider,
            DecisionProbabilitySource::LogitScoring,
            Map::new(),
        )),
        response: Some(response_info(Some(model), headers, raw)),
    })
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
    for (id, question) in &request.questions {
        let answer = answers
            .get(id)
            .ok_or_else(|| invalid("missing SGLang answer"))?;
        let (kind, labels): (&str, Vec<String>) = match question {
            DecisionQuestion::Boolean { .. } => ("yes_no", vec!["yes".into(), "no".into()]),
            DecisionQuestion::Choice { criteria, .. } => {
                ("choice", criteria.keys().cloned().collect())
            }
            DecisionQuestion::Score { criteria, .. } => (
                "score",
                (0..criteria.len()).map(|i| i.to_string()).collect(),
            ),
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
    let model = raw["model"].as_str().unwrap_or_default().to_owned();
    let server = raw["answers"].clone();
    let mut result =
        result_from_distributions(request, distributions, &model, "sglang", raw, headers)?;
    for (id, answer) in &mut result.answers {
        // Keep the server's selection and mean, checked against the same
        // distribution.
        match answer {
            DecisionAnswer::Choice { choice, .. } => {
                server[id]["choice"]
                    .as_str()
                    .ok_or_else(|| invalid("missing SGLang choice"))?
                    .clone_into(choice);
            }
            DecisionAnswer::Score { score, .. } => {
                *score = server[id]["score"]
                    .as_f64()
                    .ok_or_else(|| invalid("missing SGLang score"))?;
            }
            DecisionAnswer::Boolean { .. } => {}
        }
    }
    validate_decision_answers(&request.questions, &result.answers, None)?;
    Ok(result)
}

#[async_trait]
impl DecisionModel for RuntimeDecisionModel {
    fn provider(&self) -> &str {
        &self.provider
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn supported_question_types(&self) -> &[DecisionQuestionType] {
        DecisionQuestionType::ALL
    }
    async fn do_decide(&self, request: &DecisionCallOptions) -> Result<DecisionResult, AiMuxError> {
        let mut options = support::options(request, self.provider())?;
        let protocol = options.remove("protocol").unwrap_or(json!("systemone"));
        let images = support::take_images(&mut options)?;
        self.preflight(request, &images)?;
        match (self.provider(), protocol.as_str()) {
            ("sglang", Some("decisions")) => {
                let body = self.sglang_request(request, options, &images)?;
                let url = format!("{}/decisions", self.config.base_url()?);
                support::post(&self.config, request, &url, body, |raw, headers| {
                    sglang_response(request, raw, headers)
                })
                .await
            }
            ("sglang", Some("score")) | ("vllm", Some("generative_scoring")) => {
                crate::decision_scoring::score(self, request, options, &images).await
            }
            (_, Some("systemone")) => {
                let (url, body) = self.systemone_request(request, options, &images)?;
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
