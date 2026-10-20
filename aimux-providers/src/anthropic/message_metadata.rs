//! The result-level `providerMetadata` of a response: the Rust shape of
//! `AnthropicMessageMetadata` (`anthropic-message-metadata.ts`).

use serde_json::{Value, json};

use aimux_core::shared::provider_namespace;
use aimux_core::types::ProviderMetadata;

use crate::anthropic::language_model_options::CANONICAL as CANONICAL_KEY;

/// The result-level `providerMetadata` of a response, as `@ai-sdk/anthropic`
/// builds it: under the canonical `anthropic` key and, for a provider created
/// with another `name`, under that key too when the request used custom options.
///
/// `usage` is the raw usage object (for a stream: `message_start`'s, updated
/// by `message_delta`'s). `iterations`, `container` and `contextManagement`
/// are re-keyed in camelCase; every absent piece is `null`.
pub(crate) fn result_provider_metadata(
    options_name: &str,
    usage: &Value,
    stop_sequence: Option<&str>,
    container: Option<&Value>,
    context_management: Option<&Value>,
    used_custom_options_key: bool,
) -> ProviderMetadata {
    let field = |value: &Value, key: &str| value.get(key).cloned().unwrap_or(Value::Null);
    let iterations =
        usage
            .get("iterations")
            .and_then(Value::as_array)
            .map_or(Value::Null, |iterations| {
                Value::Array(
                    iterations
                        .iter()
                        .map(|i| {
                            let mut iteration = json!({
                                "type": field(i, "type"),
                                "inputTokens": field(i, "input_tokens"),
                                "outputTokens": field(i, "output_tokens"),
                            });
                            if let Some(model) = i.get("model").filter(|value| !value.is_null()) {
                                iteration["model"] = model.clone();
                            }
                            for (wire, key) in [
                                ("cache_creation_input_tokens", "cacheCreationInputTokens"),
                                ("cache_read_input_tokens", "cacheReadInputTokens"),
                            ] {
                                if let Some(value) = i
                                    .get(wire)
                                    .filter(|value| value.as_u64().is_some_and(|value| value != 0))
                                {
                                    iteration[key] = value.clone();
                                }
                            }
                            iteration
                        })
                        .collect(),
                )
            });
    let container = container.map_or(Value::Null, |c| {
        let skills = c
            .get("skills")
            .and_then(Value::as_array)
            .map_or(Value::Null, |skills| {
                Value::Array(
                    skills
                        .iter()
                        .map(|s| {
                            json!({
                                "type": field(s, "type"),
                                "skillId": field(s, "skill_id"),
                                "version": field(s, "version"),
                            })
                        })
                        .collect(),
                )
            });
        json!({ "expiresAt": field(c, "expires_at"), "id": field(c, "id"), "skills": skills })
    });
    let context_management = context_management.map_or(Value::Null, |cm| {
        let edits = cm
            .get("applied_edits")
            .and_then(Value::as_array)
            .map(|edits| {
                edits
                    .iter()
                    .map(|edit| match edit.as_object() {
                        Some(edit) => Value::Object(
                            edit.iter()
                                .map(|(k, v)| (snake_to_camel(k), v.clone()))
                                .collect(),
                        ),
                        None => edit.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        json!({ "appliedEdits": edits })
    });
    let metadata = json!({
        "usage": usage,
        "stopSequence": stop_sequence,
        "iterations": iterations,
        "container": container,
        "contextManagement": context_management,
    });
    let mut result =
        provider_namespace(CANONICAL_KEY, metadata).expect("provider metadata must be an object");
    if used_custom_options_key && options_name != CANONICAL_KEY {
        result.insert(options_name.to_string(), result[CANONICAL_KEY].clone());
    }
    result
}

pub(crate) fn extend_result_metadata(
    metadata: &mut ProviderMetadata,
    stop_details: Option<&Value>,
    input_transformations: Option<&Value>,
    safeguard_results: Option<&Value>,
) {
    for namespace in metadata.values_mut() {
        if let Some(details) = stop_details.filter(|details| !details.is_null()) {
            let mut mapped = json!({ "type": details["type"] });
            for (wire, key) in [
                ("category", "category"),
                ("explanation", "explanation"),
                ("recommended_model", "recommendedModel"),
            ] {
                if let Some(value) = details.get(wire).filter(|value| !value.is_null()) {
                    mapped[key] = value.clone();
                }
            }
            namespace.insert("stopDetails".to_string(), mapped);
        }
        if let Some(value) = input_transformations.filter(|value| !value.is_null()) {
            namespace.insert("inputTransformations".to_string(), value.clone());
        }
        if let Some(value) = safeguard_results.filter(|value| !value.is_null()) {
            namespace.insert("safeguardResults".to_string(), value.clone());
        }
    }
}

/// `cleared_input_tokens` -> `clearedInputTokens`.
fn snake_to_camel(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    let mut upper = false;
    for c in key.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}
