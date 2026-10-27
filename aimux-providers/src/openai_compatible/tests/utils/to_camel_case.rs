use serde_json::{Value, json};

use aimux_core::options::CallOptions;
use aimux_core::types::Warning;

use crate::openai_compatible::ChatSettings;
use crate::openai_compatible::chat::chat_language_model::{
    ChatBodySpec, RequestBodyResult, build_request_body,
};
use crate::openai_compatible::utils::to_camel_case;

/// `resolveProviderOptionsKey` and `warnIfDeprecatedProviderOptionsKey` have
/// no standalone Rust form: both run inside `build_request_body` (the
/// metadata key it reports, and the deprecation warning it raises). An empty
/// prompt with the baseline chat settings raises no other warning.
fn build(raw_name: &str, provider_options: Option<Value>) -> RequestBodyResult {
    let options = CallOptions {
        provider_options: provider_options
            .map(|value| serde_json::from_value(value).expect("provider options")),
        ..CallOptions::default()
    };
    build_request_body(
        "model",
        &options,
        false,
        &ChatBodySpec {
            provider_options_name: raw_name,
            chat: &ChatSettings::default(),
        },
    )
    .expect("request body")
}

fn resolve_provider_options_key(raw_name: &str, provider_options: Option<Value>) -> String {
    build(raw_name, provider_options).metadata_key
}

fn warn_if_deprecated_provider_options_key(
    raw_name: &str,
    provider_options: Option<Value>,
) -> Vec<Warning> {
    build(raw_name, provider_options).warnings
}

fn warnings_json(warnings: &[Warning]) -> Value {
    serde_json::to_value(warnings).expect("warnings serialize")
}

// The `toCamelCase` describe block shares the file's name.
#[allow(clippy::module_inception)]
mod to_camel_case {
    use super::*;

    /// upstream: @ai-sdk/openai-compatible src/utils/to-camel-case.test.ts › toCamelCase › should convert hyphenated names to camelCase
    #[test]
    fn should_convert_hyphenated_names_to_camel_case() {
        assert_eq!(to_camel_case("provider-name"), "providerName");
    }

    /// upstream: @ai-sdk/openai-compatible src/utils/to-camel-case.test.ts › toCamelCase › should convert underscored names to camelCase
    #[test]
    fn should_convert_underscored_names_to_camel_case() {
        assert_eq!(to_camel_case("provider_name"), "providerName");
    }

    /// upstream: @ai-sdk/openai-compatible src/utils/to-camel-case.test.ts › toCamelCase › should handle multiple separators
    #[test]
    fn should_handle_multiple_separators() {
        assert_eq!(to_camel_case("my-provider-name"), "myProviderName");
    }

    /// upstream: @ai-sdk/openai-compatible src/utils/to-camel-case.test.ts › toCamelCase › should return the same string when already camelCase
    #[test]
    fn should_return_the_same_string_when_already_camel_case() {
        assert_eq!(to_camel_case("providerName"), "providerName");
    }

    /// upstream: @ai-sdk/openai-compatible src/utils/to-camel-case.test.ts › toCamelCase › should return the same string when no separators
    #[test]
    fn should_return_the_same_string_when_no_separators() {
        assert_eq!(to_camel_case("openai"), "openai");
    }

    /// upstream: @ai-sdk/openai-compatible src/utils/to-camel-case.test.ts › toCamelCase › should handle empty string
    #[test]
    fn should_handle_empty_string() {
        assert_eq!(to_camel_case(""), "");
    }
}

mod resolve_provider_options_key {
    use super::*;

    /// upstream: @ai-sdk/openai-compatible src/utils/to-camel-case.test.ts › resolveProviderOptionsKey › should return camelCase key when camelCase options are present
    #[test]
    fn should_return_camel_case_key_when_camel_case_options_are_present() {
        assert_eq!(
            resolve_provider_options_key(
                "provider-name",
                Some(json!({ "providerName": { "someOption": "value" } })),
            ),
            "providerName"
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/utils/to-camel-case.test.ts › resolveProviderOptionsKey › should return raw key when only raw options are present
    #[test]
    fn should_return_raw_key_when_only_raw_options_are_present() {
        assert_eq!(
            resolve_provider_options_key(
                "provider-name",
                Some(json!({ "provider-name": { "someOption": "value" } })),
            ),
            "provider-name"
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/utils/to-camel-case.test.ts › resolveProviderOptionsKey › should return camelCase key when both are present
    #[test]
    fn should_return_camel_case_key_when_both_are_present() {
        assert_eq!(
            resolve_provider_options_key(
                "provider-name",
                Some(json!({
                    "provider-name": { "a": 1 },
                    "providerName": { "b": 2 },
                })),
            ),
            "providerName"
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/utils/to-camel-case.test.ts › resolveProviderOptionsKey › should return raw key when no options are present
    #[test]
    fn should_return_raw_key_when_no_options_are_present() {
        assert_eq!(
            resolve_provider_options_key("provider-name", Some(json!({}))),
            "provider-name"
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/utils/to-camel-case.test.ts › resolveProviderOptionsKey › should return raw key when providerOptions is undefined
    #[test]
    fn should_return_raw_key_when_provider_options_is_undefined() {
        assert_eq!(
            resolve_provider_options_key("provider-name", None),
            "provider-name"
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/utils/to-camel-case.test.ts › resolveProviderOptionsKey › should return raw key when name has no separators
    #[test]
    fn should_return_raw_key_when_name_has_no_separators() {
        assert_eq!(
            resolve_provider_options_key("openai", Some(json!({ "openai": { "a": 1 } }))),
            "openai"
        );
    }
}

mod warn_if_deprecated_provider_options_key {
    use super::*;

    /// upstream: @ai-sdk/openai-compatible src/utils/to-camel-case.test.ts › warnIfDeprecatedProviderOptionsKey › should push a deprecated warning when raw key is used and differs from camelCase
    #[test]
    fn should_push_a_deprecated_warning_when_raw_key_is_used_and_differs() {
        let warnings = warn_if_deprecated_provider_options_key(
            "black-forest-labs",
            Some(json!({ "black-forest-labs": { "style": "hd" } })),
        );
        assert_eq!(
            warnings_json(&warnings),
            json!([
                {
                    "type": "deprecated",
                    "setting": "providerOptions key 'black-forest-labs'",
                    "message": "Use 'blackForestLabs' instead.",
                },
            ])
        );
    }

    /// upstream: @ai-sdk/openai-compatible src/utils/to-camel-case.test.ts › warnIfDeprecatedProviderOptionsKey › should not push a warning when only camelCase key is used
    #[test]
    fn should_not_push_a_warning_when_only_camel_case_key_is_used() {
        let warnings = warn_if_deprecated_provider_options_key(
            "black-forest-labs",
            Some(json!({ "blackForestLabs": { "style": "hd" } })),
        );
        assert_eq!(warnings_json(&warnings), json!([]));
    }

    /// upstream: @ai-sdk/openai-compatible src/utils/to-camel-case.test.ts › warnIfDeprecatedProviderOptionsKey › should not push a warning when raw name is already camelCase
    #[test]
    fn should_not_push_a_warning_when_raw_name_is_already_camel_case() {
        let warnings = warn_if_deprecated_provider_options_key(
            "openai",
            Some(json!({ "openai": { "user": "test" } })),
        );
        assert_eq!(warnings_json(&warnings), json!([]));
    }

    /// upstream: @ai-sdk/openai-compatible src/utils/to-camel-case.test.ts › warnIfDeprecatedProviderOptionsKey › should not push a warning when raw key is not present in providerOptions
    #[test]
    fn should_not_push_a_warning_when_raw_key_is_not_present_in_provider_options() {
        let warnings =
            warn_if_deprecated_provider_options_key("black-forest-labs", Some(json!({})));
        assert_eq!(warnings_json(&warnings), json!([]));
    }

    /// upstream: @ai-sdk/openai-compatible src/utils/to-camel-case.test.ts › warnIfDeprecatedProviderOptionsKey › should not push a warning when providerOptions is undefined
    #[test]
    fn should_not_push_a_warning_when_provider_options_is_undefined() {
        let warnings = warn_if_deprecated_provider_options_key("black-forest-labs", None);
        assert_eq!(warnings_json(&warnings), json!([]));
    }
}

// ── Older aimux tests (previously the unit tests of `openai_compatible/convert.rs`) ──

mod camel_case {
    use crate::openai_compatible::utils::to_camel_case;

    #[test]
    fn camel_case_follows_the_ai_sdk() {
        assert_eq!(to_camel_case("acme"), "acme");
        assert_eq!(to_camel_case("my_provider"), "myProvider");
        assert_eq!(to_camel_case("my-provider"), "myProvider");
        assert_eq!(
            to_camel_case("vertex_ai_openai_models"),
            "vertexAiOpenaiModels"
        );
        assert_eq!(to_camel_case("a_1"), "a_1");
    }
}
