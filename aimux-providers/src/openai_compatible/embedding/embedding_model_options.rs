//! The provider options of the embedding model
//! (`openai-compatible-embedding-model-options.ts`):
//! `openaiCompatibleEmbeddingModelOptions`.

use aimux_core::embedding_model::EmbeddingCallOptions;
use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use aimux_provider_utils::{optional_field, parse_provider_options};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::openai_compatible::utils::to_camel_case;

/// `openaiCompatibleEmbeddingModelOptions`: a zod object of `.optional()`
/// fields, so unknown keys are dropped and an explicit `null` is rejected.
#[derive(Debug, Default, Deserialize)]
struct EmbeddingOptions {
    /// `z.number()`: any JSON number, sent as given.
    #[serde(default, deserialize_with = "optional_field")]
    dimensions: Option<serde_json::Number>,
    #[serde(default, deserialize_with = "optional_field")]
    user: Option<String>,
}

/// The `dimensions` and `user` body fields: `Object.assign` of the parsed
/// `openai-compatible` (deprecated), `openaiCompatible` and provider
/// namespaces, a field a later namespace sets winning.
///
/// # Errors
///
/// `InvalidArgument` "invalid {namespace} provider options" when a namespace
/// does not match the schema (`parseProviderOptions`).
pub(super) fn embedding_options(
    options: &EmbeddingCallOptions,
    name: &str,
    warnings: &mut Vec<Warning>,
) -> Result<Map<String, Value>, AiMuxError> {
    let all = options.provider_options.as_ref();
    let mut merged = EmbeddingOptions::default();
    for key in ["openai-compatible", "openaiCompatible", name] {
        let Some(parsed) = parse_provider_options::<EmbeddingOptions>(key, all)? else {
            continue;
        };
        if key == "openai-compatible" {
            warnings.push(Warning::Deprecated {
                setting: "providerOptions key 'openai-compatible'".into(),
                message: "Use 'openaiCompatible' instead.".into(),
            });
        }
        merged = EmbeddingOptions {
            dimensions: parsed.dimensions.or(merged.dimensions),
            user: parsed.user.or(merged.user),
        };
    }
    let camel = to_camel_case(name);
    if camel != name && all.is_some_and(|all| all.contains_key(name)) {
        warnings.push(Warning::Deprecated {
            setting: format!("providerOptions key '{name}'"),
            message: format!("Use '{camel}' instead."),
        });
    }
    let mut body = Map::new();
    if let Some(dimensions) = merged.dimensions {
        body.insert("dimensions".into(), json!(dimensions));
    }
    if let Some(user) = merged.user {
        body.insert("user".into(), json!(user));
    }
    Ok(body)
}
