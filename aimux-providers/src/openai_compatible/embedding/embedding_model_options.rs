//! The provider options of the embedding model
//! (`openai-compatible-embedding-model-options.ts`).

use aimux_core::embedding_model::EmbeddingCallOptions;
use aimux_core::error::AiMuxError;
use aimux_core::shared::Warning;
use serde_json::{Map, Value};

use crate::openai_compatible::utils::to_camel_case;

/// Validate each namespace before merging, as upstream's provider-options schema does.
pub(super) fn embedding_options(
    options: &EmbeddingCallOptions,
    name: &str,
    warnings: &mut Vec<Warning>,
) -> Result<Map<String, Value>, AiMuxError> {
    let mut merged = Map::new();
    for key in ["openai-compatible", "openaiCompatible", name] {
        let Some(object) = options
            .provider_options
            .as_ref()
            .and_then(|all| all.get(key))
        else {
            continue;
        };
        for (setting, valid) in [
            ("dimensions", Value::is_number as fn(&Value) -> bool),
            ("user", Value::is_string),
        ] {
            if let Some(value) = object.get(setting) {
                if !valid(value) {
                    return Err(AiMuxError::InvalidArgument(format!(
                        "invalid providerOptions.{key}.{setting}"
                    )));
                }
                merged.insert(setting.to_string(), value.clone());
            }
        }
    }
    if options
        .provider_options
        .as_ref()
        .is_some_and(|all| all.contains_key("openai-compatible"))
    {
        warnings.push(Warning::Deprecated {
            setting: "providerOptions key 'openai-compatible'".into(),
            message: "Use 'openaiCompatible' instead.".into(),
        });
    }
    let camel = to_camel_case(name);
    if camel != name
        && options
            .provider_options
            .as_ref()
            .is_some_and(|all| all.contains_key(name))
    {
        warnings.push(Warning::Deprecated {
            setting: format!("providerOptions key '{name}'"),
            message: format!("Use '{camel}' instead."),
        });
    }
    Ok(merged)
}
