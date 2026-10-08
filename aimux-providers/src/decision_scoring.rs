//! Explicit pre-encoded, single-token candidate scoring. The caller owns the
//! tokenizer and prompt encoding; every upstream exchange is recorded normally.
use std::collections::{BTreeMap, HashMap, HashSet};

use aimux_core::AiMuxError;
use aimux_core::decision_model::*;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::decision_support::{invalid, post};
use crate::runtime_decision::{RuntimeDecisionModel, result_from_distributions};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EncodedQuestion {
    prompt_token_ids: Vec<u32>,
    label_token_ids: Vec<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    encoded: BTreeMap<String, EncodedQuestion>,
    /// Provenance supplied by the encoder; no claim that aimux verified deployment.
    model_revision: String,
    tokenizer_revision: String,
    prompt_format_version: String,
    temperature: Option<f64>,
}

pub(crate) async fn score(
    model: &RuntimeDecisionModel,
    request: &DecisionCallOptions,
    options: Map<String, Value>,
) -> Result<DecisionResult, AiMuxError> {
    let options: Options = serde_json::from_value(Value::Object(options))
        .map_err(|e| AiMuxError::InvalidArgument(format!("candidate scoring options: {e}")))?;
    if !request.images.is_empty() {
        return Err(AiMuxError::UnsupportedFunctionality("pre-encoded scoring accepts text token IDs only; use the runtime's native decision endpoint for images".into()));
    }
    if [
        &options.model_revision,
        &options.tokenizer_revision,
        &options.prompt_format_version,
    ]
    .iter()
    .any(|s| s.trim().is_empty())
    {
        return Err(AiMuxError::InvalidArgument("candidate scoring requires model_revision, tokenizer_revision and prompt_format_version provenance".into()));
    }
    let temperature = options.temperature.unwrap_or(1.0);
    if !temperature.is_finite()
        || temperature <= 0.0
        || (model.provider() == "vllm" && temperature != 1.0)
    {
        return Err(AiMuxError::InvalidArgument(
            "scoring temperature must be positive; vLLM generative_scoring only supports 1".into(),
        ));
    }
    if options.encoded.len() != request.questions.len() {
        return Err(AiMuxError::InvalidArgument(
            "encoded questions must exactly match decision question IDs".into(),
        ));
    }
    let mut encoded = Vec::new();
    for q in &request.questions {
        let entry = options.encoded.get(q.id()).ok_or_else(|| {
            AiMuxError::InvalidArgument(format!("missing encoding for {}", q.id()))
        })?;
        let count = match q {
            DecisionQuestion::Boolean { .. } => 2,
            DecisionQuestion::Choice { options, .. } => options.len(),
            DecisionQuestion::Score { levels, .. } => levels.len(),
        };
        if entry.prompt_token_ids.is_empty()
            || entry.label_token_ids.len() != count
            || entry.label_token_ids.iter().collect::<HashSet<_>>().len() != count
        {
            return Err(AiMuxError::InvalidArgument(
                "each encoding requires a nonempty prompt and one distinct token ID per candidate"
                    .into(),
            ));
        }
        encoded.push(entry);
    }
    let base = model.config.base_url.trim_end_matches('/');
    if model.provider() == "sglang" {
        let body = json!({"model":model.model_id,"query":[],"items":encoded.iter().map(|e| &e.prompt_token_ids).collect::<Vec<_>>(),"label_token_ids":encoded.iter().map(|e| &e.label_token_ids).collect::<Vec<_>>(),"apply_softmax":true,"temperature":temperature,"return_token_logprobs":true});
        return post(
            &model.config,
            request,
            &format!("{base}/score"),
            body,
            |raw, headers| {
                let scores: Vec<Vec<f64>> = serde_json::from_value(raw["scores"].clone())
                    .map_err(|e| invalid(format!("SGLang score matrix: {e}")))?;
                result_from_distributions(
                    request,
                    scores,
                    raw["model"].as_str().unwrap_or_default().into(),
                    model.provider(),
                    raw,
                    headers,
                )
            },
        )
        .await;
    }

    // vLLM returns only the first label's normalized probability. Rotate that
    // label over the same candidate set, without changing the encoded prompt.
    let url = format!(
        "{}/generative_scoring",
        base.strip_suffix("/v1").unwrap_or(base)
    );
    let mut distributions = Vec::new();
    let mut exchanges = Vec::new();
    let mut response_model: Option<String> = None;
    let mut input_tokens = Some(0_u32);
    let mut output_tokens = Some(0_u32);
    for entry in encoded {
        let mut distribution = Vec::new();
        for index in 0..entry.label_token_ids.len() {
            let mut labels = entry.label_token_ids.clone();
            labels.rotate_left(index);
            let body = json!({"model":model.model_id,"query":[],"items":[entry.prompt_token_ids],"label_token_ids":labels,"apply_softmax":true,"add_special_tokens":false});
            let (probability, raw, headers) =
                post(&model.config, request, &url, body, |raw, headers| {
                    let data = raw["data"]
                        .as_array()
                        .ok_or_else(|| invalid("vLLM scoring omitted data"))?;
                    if data.len() != 1 || data[0]["index"] != 0 {
                        return Err(invalid("vLLM scoring returned wrong item indices"));
                    }
                    let probability = data[0]["score"]
                        .as_f64()
                        .filter(|p| (0.0..=1.0).contains(p))
                        .ok_or_else(|| invalid("vLLM scoring returned invalid probability"))?;
                    let actual = raw["model"]
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .ok_or_else(|| invalid("vLLM scoring omitted model"))?;
                    if response_model.as_deref().is_some_and(|m| m != actual) {
                        return Err(invalid("vLLM model changed between candidate reads"));
                    }
                    response_model = Some(actual.into());
                    Ok((probability, raw, headers))
                })
                .await?;
            let usage = &raw["usage"];
            input_tokens = add_usage(input_tokens, usage.get("prompt_tokens"));
            output_tokens = add_usage(output_tokens, usage.get("completion_tokens"));
            distribution.push(probability);
            exchanges.push(json!({"headers":headers,"body":raw}));
        }
        distributions.push(distribution);
    }
    // Explicit aggregate, not a fabricated upstream response. Original bodies
    // and headers also remain separate HTTP records in the unified recorder.
    let raw = json!({"exchanges":exchanges,"usage":{"prompt_tokens":input_tokens,"completion_tokens":output_tokens}});
    result_from_distributions(
        request,
        distributions,
        response_model.unwrap_or_default(),
        model.provider(),
        raw,
        HashMap::new(),
    )
}

fn add_usage(total: Option<u32>, value: Option<&Value>) -> Option<u32> {
    total?.checked_add(value?.as_u64()?.try_into().ok()?)
}
