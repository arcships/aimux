//! Remaining Google provider tests — ported from the TS SDK test suite.
//!
//! Covers the pure-function and config tests that were not already exercised
//! by `google_model_test.rs` / `google_provider_tools_test.rs`:
//!
//! - get-model-path.test.ts -> get_model_path tests
//! - google-supported-file-url.test.ts -> is_supported_file_url tests
//! - google-model-capabilities.test.ts -> get_google_model_capabilities
//! - convert-json-schema-to-openapi-schema.test.ts -> schema conversion
//! - google-json-accumulator.test.ts -> GoogleJsonAccumulator
//! - google-provider.test.ts -> provider config (env var, headers, base URL)
//!
//! Reference: reference/ai/packages/google/src/.

use serde_json::{Value, json};
use serial_test::serial;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use aimux_core::content::ContentPart;
use aimux_core::error::AiMuxError;
use aimux_core::language_model::LanguageModel;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::CallOptions;
use aimux_providers::google::convert::{
    convert_json_schema_to_openapi_schema, get_google_model_capabilities,
};
use aimux_providers::google::utils::{
    GoogleJsonAccumulator, PartialArg, get_model_path, is_supported_file_url,
};
use aimux_providers::{GoogleConfig, GoogleProvider};

// ════════════════════════════════════════════════════════════════════════════
// get_model_path  (TS: get-model-path.test.ts)
// ════════════════════════════════════════════════════════════════════════════

mod get_model_path_tests {
    use super::*;

    #[test]
    fn pass_through_for_models_slash() {
        assert_eq!(get_model_path("models/some-model"), "models/some-model");
    }

    #[test]
    fn pass_through_for_tuned_models_slash() {
        assert_eq!(
            get_model_path("tunedModels/some-model"),
            "tunedModels/some-model"
        );
    }

    #[test]
    fn add_prefix_to_models_without_slash() {
        assert_eq!(get_model_path("some-model"), "models/some-model");
    }
}

// ════════════════════════════════════════════════════════════════════════════
// is_supported_file_url  (TS: google-supported-file-url.test.ts)
// ════════════════════════════════════════════════════════════════════════════

mod is_supported_file_url_tests {
    use super::*;

    #[test]
    fn valid_google_file_urls() {
        assert!(is_supported_file_url(
            "https://generativelanguage.googleapis.com/v1beta/files/00000000-00000000-00000000-00000000"
        ));
        assert!(is_supported_file_url(
            "https://generativelanguage.googleapis.com/v1beta/files/test123"
        ));
    }

    #[test]
    fn valid_youtube_urls() {
        let valid = [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&feature=youtu.be",
            "https://youtu.be/dQw4w9WgXcQ?t=42",
        ];
        for url in valid {
            assert!(is_supported_file_url(url), "should be supported: {}", url);
        }
    }

    #[test]
    fn invalid_youtube_urls() {
        let invalid = [
            "https://youtube.com/channel/UCdQw4w9WgXcQ",
            "https://youtube.com/playlist?list=PLdQw4w9WgXcQ",
            "https://m.youtube.com/watch?v=dQw4w9WgXcQ",
            "http://youtube.com/watch?v=dQw4w9WgXcQ",
            "https://vimeo.com/123456789",
        ];
        for url in invalid {
            assert!(
                !is_supported_file_url(url),
                "should NOT be supported: {}",
                url
            );
        }
    }

    #[test]
    fn non_google_file_urls() {
        let cases = [
            "https://example.com",
            "https://example.com/foo/bar",
            "https://generativelanguage.googleapis.com",
            "https://generativelanguage.googleapis.com/v1/other",
            "http://generativelanguage.googleapis.com/v1beta/files/test",
            "https://api.googleapis.com/v1beta/files/test",
        ];
        for url in cases {
            assert!(
                !is_supported_file_url(url),
                "should NOT be supported: {}",
                url
            );
        }
    }
}

// ════════════════════════════════════════════════════════════════════════════
// get_google_model_capabilities  (TS: google-model-capabilities.test.ts)
// ════════════════════════════════════════════════════════════════════════════

mod model_capabilities_tests {
    use super::*;

    #[test]
    fn gemini_pro_is_legacy() {
        let caps = get_google_model_capabilities("gemini-pro");
        assert!(!caps.supports_gemini_2_tools);
        assert!(!caps.supports_file_search);
        assert!(!caps.uses_gemini_3_features);
    }

    #[test]
    fn gemini_pro_vision_is_legacy() {
        let caps = get_google_model_capabilities("gemini-pro-vision");
        assert!(!caps.supports_gemini_2_tools);
        assert!(!caps.supports_file_search);
        assert!(!caps.uses_gemini_3_features);
    }

    #[test]
    fn gemini_1_5_flash_is_legacy() {
        let caps = get_google_model_capabilities("gemini-1.5-flash");
        assert!(!caps.supports_gemini_2_tools);
        assert!(!caps.supports_file_search);
        assert!(!caps.uses_gemini_3_features);
    }

    #[test]
    fn gemini_robotics_er_1_5_is_legacy() {
        let caps = get_google_model_capabilities("gemini-robotics-er-1.5-preview");
        assert!(!caps.supports_gemini_2_tools);
        assert!(!caps.supports_file_search);
        assert!(!caps.uses_gemini_3_features);
    }

    #[test]
    fn gemini_2_0_flash_supports_gemini_2_tools() {
        let caps = get_google_model_capabilities("gemini-2.0-flash");
        assert!(caps.supports_gemini_2_tools);
        assert!(!caps.supports_file_search);
        assert!(!caps.uses_gemini_3_features);
    }

    #[test]
    fn gemini_2_5_flash_supports_file_search() {
        let caps = get_google_model_capabilities("gemini-2.5-flash");
        assert!(caps.supports_gemini_2_tools);
        assert!(caps.supports_file_search);
        assert!(!caps.uses_gemini_3_features);
    }

    #[test]
    fn gemini_3_1_pro_uses_gemini_3_features() {
        let caps = get_google_model_capabilities("gemini-3.1-pro-preview");
        assert!(caps.supports_gemini_2_tools);
        assert!(caps.supports_file_search);
        assert!(caps.uses_gemini_3_features);
    }

    #[test]
    fn unknown_future_gemini_inherits_newest() {
        let caps = get_google_model_capabilities("gemini-99-pro-preview");
        assert!(caps.supports_gemini_2_tools);
        assert!(caps.supports_file_search);
        assert!(caps.uses_gemini_3_features);
    }

    #[test]
    fn gemini_ultra_latest_uses_gemini_3_features() {
        let caps = get_google_model_capabilities("gemini-ultra-latest");
        assert!(caps.supports_gemini_2_tools);
        assert!(caps.supports_file_search);
        assert!(caps.uses_gemini_3_features);
    }

    #[test]
    fn nano_banana_supports_gemini_2_tools_only() {
        let caps = get_google_model_capabilities("nano-banana-pro-preview");
        assert!(caps.supports_gemini_2_tools);
        assert!(!caps.supports_file_search);
        assert!(!caps.uses_gemini_3_features);
    }
}

// ════════════════════════════════════════════════════════════════════════════
// convert_json_schema_to_openapi_schema  (TS: convert-json-schema-to-openapi-schema.test.ts)
// ════════════════════════════════════════════════════════════════════════════

mod schema_conversion_tests {
    use super::*;

    fn convert_root(schema: &Value) -> Value {
        convert_json_schema_to_openapi_schema(schema, true)
    }

    fn convert_nested(schema: &Value) -> Value {
        convert_json_schema_to_openapi_schema(schema, false)
    }

    #[test]
    fn removes_additional_properties_and_schema() {
        let input = json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "number" }
            },
            "additionalProperties": false
        });
        assert_eq!(
            convert_root(&input),
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "age": { "type": "number" }
                }
            })
        );
    }

    #[test]
    fn removes_additional_properties_from_nested_objects() {
        let input = json!({
            "type": "object",
            "properties": {
                "keys": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                    "description": "Description for the key"
                }
            },
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#"
        });
        assert_eq!(
            convert_root(&input),
            json!({
                "type": "object",
                "properties": {
                    "keys": {
                        "type": "object",
                        "description": "Description for the key"
                    }
                }
            })
        );
    }

    #[test]
    fn handles_nested_objects_and_arrays() {
        let input = json!({
            "type": "object",
            "properties": {
                "users": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "number" },
                            "name": { "type": "string" }
                        },
                        "additionalProperties": false
                    }
                }
            },
            "additionalProperties": false
        });
        assert_eq!(
            convert_root(&input),
            json!({
                "type": "object",
                "properties": {
                    "users": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "number" },
                                "name": { "type": "string" }
                            }
                        }
                    }
                }
            })
        );
    }

    #[test]
    fn converts_const_to_enum_single_value() {
        let input = json!({
            "type": "object",
            "properties": { "status": { "const": "active" } }
        });
        assert_eq!(
            convert_root(&input),
            json!({
                "type": "object",
                "properties": { "status": { "enum": ["active"] } }
            })
        );
    }

    #[test]
    fn handles_all_of_any_of_one_of() {
        let input = json!({
            "type": "object",
            "properties": {
                "allOfProp": { "allOf": [{ "type": "string" }, { "minLength": 5 }] },
                "anyOfProp": { "anyOf": [{ "type": "string" }, { "type": "number" }] },
                "oneOfProp": { "oneOf": [{ "type": "boolean" }, { "type": "null" }] }
            }
        });
        assert_eq!(
            convert_root(&input),
            json!({
                "type": "object",
                "properties": {
                    "allOfProp": { "allOf": [{ "type": "string" }, { "minLength": 5 }] },
                    "anyOfProp": { "anyOf": [{ "type": "string" }, { "type": "number" }] },
                    "oneOfProp": { "oneOf": [{ "type": "boolean" }, { "type": "null" }] }
                }
            })
        );
    }

    #[test]
    fn preserves_format() {
        let input = json!({
            "type": "object",
            "properties": { "timestamp": { "type": "string", "format": "date-time" } }
        });
        assert_eq!(
            convert_root(&input),
            json!({
                "type": "object",
                "properties": { "timestamp": { "type": "string", "format": "date-time" } }
            })
        );
    }

    #[test]
    fn handles_required_properties() {
        let input = json!({
            "type": "object",
            "properties": { "id": { "type": "number" }, "name": { "type": "string" } },
            "required": ["id"]
        });
        assert_eq!(
            convert_root(&input),
            json!({
                "type": "object",
                "properties": { "id": { "type": "number" }, "name": { "type": "string" } },
                "required": ["id"]
            })
        );
    }

    #[test]
    fn deeply_nested_const_becomes_enum() {
        let input = json!({
            "type": "object",
            "properties": {
                "nested": {
                    "type": "object",
                    "properties": {
                        "deeplyNested": {
                            "anyOf": [
                                { "type": "object", "properties": { "value": { "const": "specific value" } } },
                                { "type": "string" }
                            ]
                        }
                    }
                }
            }
        });
        assert_eq!(
            convert_root(&input),
            json!({
                "type": "object",
                "properties": {
                    "nested": {
                        "type": "object",
                        "properties": {
                            "deeplyNested": {
                                "anyOf": [
                                    { "type": "object", "properties": { "value": { "enum": ["specific value"] } } },
                                    { "type": "string" }
                                ]
                            }
                        }
                    }
                }
            })
        );
    }

    #[test]
    fn complex_schema_with_nested_const_and_anyof() {
        let input = json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "number" },
                "contact": {
                    "anyOf": [
                        {
                            "type": "object",
                            "properties": {
                                "type": { "type": "string", "const": "email" },
                                "value": { "type": "string" }
                            },
                            "required": ["type", "value"],
                            "additionalProperties": false
                        },
                        {
                            "type": "object",
                            "properties": {
                                "type": { "type": "string", "const": "phone" },
                                "value": { "type": "string" }
                            },
                            "required": ["type", "value"],
                            "additionalProperties": false
                        }
                    ]
                }
            },
            "required": ["name", "age", "contact"],
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#"
        });
        let result = convert_root(&input);
        assert_eq!(result["type"], "object");
        assert_eq!(result["required"], json!(["name", "age", "contact"]));
        let contact_anyof = &result["properties"]["contact"]["anyOf"];
        assert_eq!(
            contact_anyof[0]["properties"]["type"]["enum"],
            json!(["email"])
        );
        assert_eq!(
            contact_anyof[1]["properties"]["type"]["enum"],
            json!(["phone"])
        );
    }

    #[test]
    fn handles_null_type_correctly() {
        let input = json!({
            "type": "object",
            "properties": {
                "nullableField": { "type": ["string", "null"] },
                "explicitNullField": { "type": "null" }
            }
        });
        let result = convert_root(&input);
        assert_eq!(
            result["properties"]["nullableField"],
            json!({ "anyOf": [{ "type": "string" }], "nullable": true })
        );
        assert_eq!(
            result["properties"]["explicitNullField"],
            json!({ "type": "null" })
        );
    }

    #[test]
    fn handles_descriptions() {
        let input = json!({
            "type": "object",
            "description": "A user object",
            "properties": {
                "id": { "type": "number", "description": "The user ID" },
                "name": { "type": "string", "description": "The user's full name" },
                "email": { "type": "string", "format": "email", "description": "The user's email address" }
            },
            "required": ["id", "name"]
        });
        assert_eq!(convert_root(&input), input);
    }

    #[test]
    fn returns_null_for_empty_object_schemas_at_root() {
        let cases = [
            json!({ "type": "object" }),
            json!({ "type": "object", "properties": {} }),
        ];
        for schema in &cases {
            assert!(
                convert_root(schema).is_null(),
                "expected null for {}",
                schema
            );
        }
    }

    #[test]
    fn preserves_nested_empty_object_with_description() {
        let input = json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "URL to navigate to" },
                "launchOptions": { "type": "object", "description": "PuppeteerJS LaunchOptions" },
                "allowDangerous": { "type": "boolean", "description": "Allow dangerous options" }
            },
            "required": ["url", "launchOptions"]
        });
        assert_eq!(convert_root(&input), input);
    }

    #[test]
    fn preserves_nested_empty_object_without_description() {
        let input = json!({
            "type": "object",
            "properties": { "options": { "type": "object" } },
            "required": ["options"]
        });
        assert_eq!(convert_root(&input), input);
    }

    #[test]
    fn handles_non_empty_object_schemas() {
        let input = json!({
            "type": "object",
            "properties": { "name": { "type": "string" } }
        });
        assert_eq!(convert_root(&input), input);
    }

    #[test]
    fn converts_string_enum_properties() {
        let input = json!({
            "type": "object",
            "properties": {
                "kind": { "type": "string", "enum": ["text", "code", "image"] }
            },
            "required": ["kind"],
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#"
        });
        assert_eq!(
            convert_root(&input),
            json!({
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["text", "code", "image"] }
                },
                "required": ["kind"]
            })
        );
    }

    #[test]
    fn converts_nullable_string_enum() {
        let input = json!({
            "type": "object",
            "properties": {
                "fieldD": {
                    "anyOf": [
                        { "type": "string", "enum": ["a", "b", "c"] },
                        { "type": "null" }
                    ]
                }
            },
            "required": ["fieldD"],
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#"
        });
        let result = convert_root(&input);
        // The anyOf with a null branch collapses into nullable + the non-null
        // schema's fields.
        assert_eq!(
            result["properties"]["fieldD"],
            json!({ "nullable": true, "type": "string", "enum": ["a", "b", "c"] })
        );
    }

    #[test]
    fn type_array_multiple_non_null_plus_null() {
        let input = json!({
            "type": "object",
            "properties": {
                "multiTypeField": { "type": ["string", "number", "null"] }
            }
        });
        assert_eq!(
            convert_root(&input),
            json!({
                "type": "object",
                "properties": {
                    "multiTypeField": {
                        "anyOf": [{ "type": "string" }, { "type": "number" }],
                        "nullable": true
                    }
                }
            })
        );
    }

    #[test]
    fn type_array_without_null_becomes_anyof() {
        let input = json!({
            "type": "object",
            "properties": {
                "multiTypeField": { "type": ["string", "number"] }
            }
        });
        assert_eq!(
            convert_root(&input),
            json!({
                "type": "object",
                "properties": {
                    "multiTypeField": { "anyOf": [{ "type": "string" }, { "type": "number" }] }
                }
            })
        );
    }

    #[test]
    fn boolean_schema_becomes_object() {
        let input = json!(true);
        assert_eq!(
            convert_root(&input),
            json!({ "type": "boolean", "properties": {} })
        );
    }

    #[test]
    fn nested_empty_object_root_returns_null_nested_returns_type_object() {
        let empty = json!({ "type": "object", "properties": {} });
        assert!(convert_root(&empty).is_null());
        // When nested, it becomes { "type": "object" }.
        assert_eq!(convert_nested(&empty), json!({ "type": "object" }));
    }
}

// ════════════════════════════════════════════════════════════════════════════
// GoogleJsonAccumulator  (TS: google-json-accumulator.test.ts)
// ════════════════════════════════════════════════════════════════════════════

mod json_accumulator_tests {
    use super::*;

    fn s(path: &str, value: &str) -> PartialArg {
        PartialArg {
            json_path: path.to_string(),
            string_value: Some(value.to_string()),
            ..Default::default()
        }
    }

    fn s_continue(path: &str, value: &str) -> PartialArg {
        PartialArg {
            json_path: path.to_string(),
            string_value: Some(value.to_string()),
            will_continue: Some(true),
            ..Default::default()
        }
    }

    fn n(path: &str, value: f64) -> PartialArg {
        PartialArg {
            json_path: path.to_string(),
            number_value: Some(value),
            ..Default::default()
        }
    }

    fn b(path: &str, value: bool) -> PartialArg {
        PartialArg {
            json_path: path.to_string(),
            bool_value: Some(value),
            ..Default::default()
        }
    }

    fn null_arg(path: &str) -> PartialArg {
        PartialArg {
            json_path: path.to_string(),
            null_value: Some(()),
            ..Default::default()
        }
    }

    // ── flat paths ─────────────────────────────────────────────────────────

    #[test]
    fn accumulate_simple_string_with_will_continue() {
        let mut acc = GoogleJsonAccumulator::new();
        let r = acc
            .process_partial_args(&[s_continue("$.location", "Boston")])
            .unwrap();
        assert_eq!(r.current_json, json!({ "location": "Boston" }));
        assert_eq!(r.text_delta, "{\"location\":\"Boston");
    }

    #[test]
    fn continue_string_across_chunks() {
        let mut acc = GoogleJsonAccumulator::new();
        acc.process_partial_args(&[s_continue("$.location", "Boston")])
            .unwrap();
        let r = acc
            .process_partial_args(&[s("$.location", ", MA")])
            .unwrap();
        assert_eq!(r.current_json, json!({ "location": "Boston, MA" }));
        assert_eq!(r.text_delta, ", MA");
    }

    #[test]
    fn accumulate_complete_string() {
        let mut acc = GoogleJsonAccumulator::new();
        let r = acc
            .process_partial_args(&[s("$.location", "Boston")])
            .unwrap();
        assert_eq!(r.current_json, json!({ "location": "Boston" }));
        assert_eq!(r.text_delta, "{\"location\":\"Boston\"");
    }

    #[test]
    fn accumulate_number() {
        let mut acc = GoogleJsonAccumulator::new();
        let r = acc
            .process_partial_args(&[n("$.brightness", 50.0)])
            .unwrap();
        assert_eq!(r.current_json, json!({ "brightness": 50 }));
        assert_eq!(r.text_delta, "{\"brightness\":50");
    }

    #[test]
    fn accumulate_boolean() {
        let mut acc = GoogleJsonAccumulator::new();
        let r = acc.process_partial_args(&[b("$.enabled", true)]).unwrap();
        assert_eq!(r.current_json, json!({ "enabled": true }));
        assert_eq!(r.text_delta, "{\"enabled\":true");
    }

    #[test]
    fn accumulate_null() {
        let mut acc = GoogleJsonAccumulator::new();
        let r = acc.process_partial_args(&[null_arg("$.nickname")]).unwrap();
        assert_eq!(r.current_json, json!({ "nickname": null }));
        assert_eq!(r.text_delta, "{\"nickname\":null");
    }

    #[test]
    fn accumulate_multiple_args_with_commas() {
        let mut acc = GoogleJsonAccumulator::new();
        let first = acc
            .process_partial_args(&[n("$.brightness", 50.0)])
            .unwrap();
        assert_eq!(first.text_delta, "{\"brightness\":50");

        let second = acc.process_partial_args(&[b("$.enabled", true)]).unwrap();
        assert_eq!(
            second.current_json,
            json!({ "brightness": 50, "enabled": true })
        );
        assert_eq!(second.text_delta, ",\"enabled\":true");
    }

    #[test]
    fn accumulate_multiple_args_single_call() {
        let mut acc = GoogleJsonAccumulator::new();
        let r = acc
            .process_partial_args(&[
                n("$.brightness", 50.0),
                b("$.enabled", false),
                null_arg("$.nickname"),
            ])
            .unwrap();
        assert_eq!(
            r.current_json,
            json!({ "brightness": 50, "enabled": false, "nickname": null })
        );
        assert_eq!(
            r.text_delta,
            "{\"brightness\":50,\"enabled\":false,\"nickname\":null"
        );
    }

    #[test]
    fn escape_special_chars_in_continued_strings() {
        let mut acc = GoogleJsonAccumulator::new();
        acc.process_partial_args(&[s_continue("$.query", "Boston \"Lo")])
            .unwrap();
        let r = acc.process_partial_args(&[s("$.query", "gan\"")]).unwrap();
        assert_eq!(r.current_json, json!({ "query": "Boston \"Logan\"" }));
        // The continuation delta is the escaped inner content of `gan"`.
        assert_eq!(r.text_delta, "gan\\\"");
    }

    #[test]
    fn skip_args_with_empty_path() {
        let mut acc = GoogleJsonAccumulator::new();
        let r = acc.process_partial_args(&[s("$.", "ignored")]).unwrap();
        assert_eq!(r.current_json, json!({}));
        assert_eq!(r.text_delta, "");
    }

    #[test]
    fn skip_args_with_no_resolvable_value() {
        let mut acc = GoogleJsonAccumulator::new();
        let r = acc
            .process_partial_args(&[PartialArg {
                json_path: "$.something".to_string(),
                ..Default::default()
            }])
            .unwrap();
        assert_eq!(r.current_json, json!({}));
        assert_eq!(r.text_delta, "");
    }

    /// Regression (audit finding on H3): a malformed empty path like `$.[]`
    /// parses to zero segments; it must be skipped instead of panicking with a
    /// slice underflow in `emit_navigation_to`.
    #[test]
    fn malformed_empty_path_is_skipped_not_panicked() {
        let mut acc = GoogleJsonAccumulator::new();
        let r = acc
            .process_partial_args(&[
                PartialArg {
                    json_path: "$.[]".to_string(),
                    string_value: Some("x".to_string()),
                    ..Default::default()
                },
                PartialArg {
                    json_path: "$.ok".to_string(),
                    string_value: Some("y".to_string()),
                    ..Default::default()
                },
            ])
            .unwrap();
        // The malformed arg is ignored; the well-formed one still lands.
        assert_eq!(r.current_json, json!({ "ok": "y" }));
    }

    /// Regression (audit round 2): an oversized array index must produce an
    /// error instead of expanding an array with ~1e6+ Null entries (OOM).
    #[test]
    fn oversized_array_index_is_rejected() {
        let mut acc = GoogleJsonAccumulator::new();
        let err = acc
            .process_partial_args(&[PartialArg {
                json_path: "$.a[1000000]".to_string(),
                string_value: Some("x".to_string()),
                ..Default::default()
            }])
            .unwrap_err();
        assert!(matches!(err, AiMuxError::Json(_)));
    }

    /// Regression (audit round 3, A1): an oversized **intermediate** index
    /// (not the last segment) must also be rejected — previously only the
    /// leaf segment was checked, so `$.a[1000000000].b` could still force
    /// ~1 GiB of array allocation before failing.
    #[test]
    fn intermediate_oversized_index_is_rejected_atomically() {
        let mut acc = GoogleJsonAccumulator::new();
        let err = acc
            .process_partial_args(&[PartialArg {
                json_path: "$.a[1000000].b".to_string(),
                string_value: Some("x".to_string()),
                ..Default::default()
            }])
            .unwrap_err();
        assert!(matches!(err, AiMuxError::Json(_)));

        // Error atomicity: the rejected arg must not leave partially-built
        // containers (`a: []`) behind — a subsequent valid write lands cleanly.
        let r = acc
            .process_partial_args(&[PartialArg {
                json_path: "$.ok".to_string(),
                string_value: Some("y".to_string()),
                ..Default::default()
            }])
            .unwrap();
        assert_eq!(r.current_json, json!({ "ok": "y" }));
    }

    /// Regression (audit round 2): an overly deep path must be rejected.
    #[test]
    fn overdeep_path_is_rejected() {
        let mut acc = GoogleJsonAccumulator::new();
        let deep = format!("${}", ".a".repeat(100));
        let err = acc
            .process_partial_args(&[PartialArg {
                json_path: deep,
                string_value: Some("x".to_string()),
                ..Default::default()
            }])
            .unwrap_err();
        assert!(matches!(err, AiMuxError::Json(_)));
    }

    #[test]
    fn empty_partial_args_returns_empty_delta() {
        let mut acc = GoogleJsonAccumulator::new();
        let r = acc.process_partial_args(&[]).unwrap();
        assert_eq!(r.current_json, json!({}));
        assert_eq!(r.text_delta, "");
    }

    // ── nested paths ───────────────────────────────────────────────────────

    #[test]
    fn build_nested_object_from_dotted_path() {
        let mut acc = GoogleJsonAccumulator::new();
        let r = acc
            .process_partial_args(&[s("$.recipe.name", "Lasagna")])
            .unwrap();
        assert_eq!(r.current_json, json!({ "recipe": { "name": "Lasagna" } }));
        assert_eq!(r.text_delta, "{\"recipe\":{\"name\":\"Lasagna\"");
    }

    #[test]
    fn build_nested_object_with_array_from_indexed_path() {
        let mut acc = GoogleJsonAccumulator::new();
        let amount = acc
            .process_partial_args(&[s("$.recipe.ingredients[0].amount", "16 oz")])
            .unwrap();
        assert_eq!(
            amount.current_json,
            json!({ "recipe": { "ingredients": [{ "amount": "16 oz" }] } })
        );
        assert_eq!(
            amount.text_delta,
            "{\"recipe\":{\"ingredients\":[{\"amount\":\"16 oz\""
        );

        let name = acc
            .process_partial_args(&[s("$.recipe.ingredients[0].name", "Lasagna noodles")])
            .unwrap();
        assert_eq!(
            name.current_json,
            json!({ "recipe": { "ingredients": [{ "amount": "16 oz", "name": "Lasagna noodles" }] } })
        );
        assert_eq!(name.text_delta, ",\"name\":\"Lasagna noodles\"");
    }

    #[test]
    fn accumulate_multiple_array_elements_across_chunks() {
        let mut acc = GoogleJsonAccumulator::new();
        let mut deltas = Vec::new();

        let r = acc
            .process_partial_args(&[s("$.recipe.ingredients[0].amount", "16 oz")])
            .unwrap();
        deltas.push(r.text_delta.clone());

        let r = acc
            .process_partial_args(&[s("$.recipe.ingredients[0].name", "Noodles")])
            .unwrap();
        deltas.push(r.text_delta.clone());

        let r = acc
            .process_partial_args(&[s("$.recipe.ingredients[1].amount", "1 lb")])
            .unwrap();
        deltas.push(r.text_delta.clone());
        assert_eq!(r.text_delta, "},{\"amount\":\"1 lb\"");

        let r = acc
            .process_partial_args(&[s("$.recipe.ingredients[1].name", "Beef")])
            .unwrap();
        deltas.push(r.text_delta.clone());
        assert_eq!(r.text_delta, ",\"name\":\"Beef\"");

        assert_eq!(
            r.current_json,
            json!({
                "recipe": {
                    "ingredients": [
                        { "amount": "16 oz", "name": "Noodles" },
                        { "amount": "1 lb", "name": "Beef" }
                    ]
                }
            })
        );

        let f = acc.finalize();
        deltas.push(f.closing_delta);
        assert_eq!(deltas.join(""), f.final_json);
    }

    #[test]
    fn string_continuation_on_nested_paths() {
        let mut acc = GoogleJsonAccumulator::new();
        let start = acc
            .process_partial_args(&[s_continue("$.recipe.steps[0]", "Preheat oven")])
            .unwrap();
        assert_eq!(start.text_delta, "{\"recipe\":{\"steps\":[\"Preheat oven");

        let cont = acc
            .process_partial_args(&[s("$.recipe.steps[0]", " to 375°F.")])
            .unwrap();
        assert_eq!(
            cont.current_json,
            json!({ "recipe": { "steps": ["Preheat oven to 375°F."] } })
        );
        assert_eq!(cont.text_delta, " to 375°F.");
    }

    #[test]
    fn mixed_nested_and_flat_paths() {
        let mut acc = GoogleJsonAccumulator::new();
        let loc = acc
            .process_partial_args(&[s("$.location", "Boston")])
            .unwrap();
        assert_eq!(loc.text_delta, "{\"location\":\"Boston\"");

        let details = acc
            .process_partial_args(&[s("$.details.zip", "02101")])
            .unwrap();
        assert_eq!(
            details.current_json,
            json!({ "details": { "zip": "02101" }, "location": "Boston" })
        );
        assert_eq!(details.text_delta, ",\"details\":{\"zip\":\"02101\"");

        let f = acc.finalize();
        assert_eq!(f.closing_delta, "}}");
        assert_eq!(
            f.final_json,
            "{\"location\":\"Boston\",\"details\":{\"zip\":\"02101\"}}"
        );
    }

    #[test]
    fn array_elements_direct_string_values() {
        let mut acc = GoogleJsonAccumulator::new();
        let first = acc
            .process_partial_args(&[s("$.steps[0]", "Step one")])
            .unwrap();
        assert_eq!(first.text_delta, "{\"steps\":[\"Step one\"");

        let second = acc
            .process_partial_args(&[s("$.steps[1]", "Step two")])
            .unwrap();
        assert_eq!(
            second.current_json,
            json!({ "steps": ["Step one", "Step two"] })
        );
        assert_eq!(second.text_delta, ",\"Step two\"");
    }

    #[test]
    fn deeply_nested_paths() {
        let mut acc = GoogleJsonAccumulator::new();
        let r = acc.process_partial_args(&[s("$.a.b.c.d", "deep")]).unwrap();
        assert_eq!(
            r.current_json,
            json!({ "a": { "b": { "c": { "d": "deep" } } } })
        );
        assert_eq!(r.text_delta, "{\"a\":{\"b\":{\"c\":{\"d\":\"deep\"");

        let f = acc.finalize();
        assert_eq!(f.closing_delta, "}}}}");
        assert_eq!(f.final_json, "{\"a\":{\"b\":{\"c\":{\"d\":\"deep\"}}}}");
    }

    // ── finalize ───────────────────────────────────────────────────────────

    #[test]
    fn finalize_continued_string() {
        let mut acc = GoogleJsonAccumulator::new();
        acc.process_partial_args(&[s_continue("$.location", "Boston")])
            .unwrap();
        let f = acc.finalize();
        assert_eq!(f.closing_delta, "\"}");
        assert_eq!(f.final_json, "{\"location\":\"Boston\"}");
    }

    #[test]
    fn finalize_complete_string() {
        let mut acc = GoogleJsonAccumulator::new();
        acc.process_partial_args(&[s("$.location", "Boston")])
            .unwrap();
        let f = acc.finalize();
        assert_eq!(f.closing_delta, "}");
        assert_eq!(f.final_json, "{\"location\":\"Boston\"}");
    }

    #[test]
    fn finalize_multiple_args() {
        let mut acc = GoogleJsonAccumulator::new();
        acc.process_partial_args(&[n("$.brightness", 50.0), b("$.enabled", true)])
            .unwrap();
        let f = acc.finalize();
        assert_eq!(f.closing_delta, "}");
        assert_eq!(f.final_json, "{\"brightness\":50,\"enabled\":true}");
    }

    #[test]
    fn finalize_continued_string_with_continuation() {
        let mut acc = GoogleJsonAccumulator::new();
        acc.process_partial_args(&[s_continue("$.location", "Boston")])
            .unwrap();
        acc.process_partial_args(&[s("$.location", ", MA")])
            .unwrap();
        let f = acc.finalize();
        assert_eq!(f.closing_delta, "\"}");
        assert_eq!(f.final_json, "{\"location\":\"Boston, MA\"}");
    }

    #[test]
    fn finalize_empty_accumulator() {
        let acc = GoogleJsonAccumulator::new();
        let f = acc.finalize();
        assert_eq!(f.closing_delta, "{}");
        assert_eq!(f.final_json, "{}");
    }

    #[test]
    fn finalize_nested_structure() {
        let mut acc = GoogleJsonAccumulator::new();
        acc.process_partial_args(&[s("$.recipe.ingredients[0].name", "Noodles")])
            .unwrap();
        acc.process_partial_args(&[s("$.recipe.name", "Lasagna")])
            .unwrap();
        let f = acc.finalize();
        let parsed: Value = serde_json::from_str(&f.final_json).unwrap();
        assert_eq!(
            parsed,
            json!({
                "recipe": {
                    "ingredients": [{ "name": "Noodles" }],
                    "name": "Lasagna"
                }
            })
        );
    }

    #[test]
    fn finalize_nested_arrays_with_string_continuation() {
        let mut acc = GoogleJsonAccumulator::new();
        acc.process_partial_args(&[s_continue("$.recipe.steps[0]", "Preheat")])
            .unwrap();
        acc.process_partial_args(&[s("$.recipe.steps[0]", " oven.")])
            .unwrap();
        acc.process_partial_args(&[s("$.recipe.steps[1]", "Cook.")])
            .unwrap();
        let f = acc.finalize();
        let parsed: Value = serde_json::from_str(&f.final_json).unwrap();
        assert_eq!(
            parsed,
            json!({
                "recipe": { "steps": ["Preheat oven.", "Cook."] }
            })
        );
    }

    // ── concatenation invariant ────────────────────────────────────────────

    #[test]
    fn flat_args_concat_invariant() {
        let mut acc = GoogleJsonAccumulator::new();
        let mut deltas = Vec::new();

        let r = acc
            .process_partial_args(&[n("$.brightness", 50.0)])
            .unwrap();
        deltas.push(r.text_delta);

        let r = acc.process_partial_args(&[b("$.enabled", true)]).unwrap();
        deltas.push(r.text_delta);

        let r = acc.process_partial_args(&[s("$.name", "test")]).unwrap();
        deltas.push(r.text_delta);

        let f = acc.finalize();
        deltas.push(f.closing_delta);
        assert_eq!(deltas.join(""), f.final_json);
        let parsed: Value = serde_json::from_str(&f.final_json).unwrap();
        assert_eq!(
            parsed,
            json!({ "brightness": 50, "enabled": true, "name": "test" })
        );
    }

    #[test]
    fn nested_args_concat_invariant() {
        let mut acc = GoogleJsonAccumulator::new();
        let mut deltas = Vec::new();

        let r = acc
            .process_partial_args(&[s("$.recipe.ingredients[0].amount", "16 oz")])
            .unwrap();
        deltas.push(r.text_delta);
        let r = acc
            .process_partial_args(&[s("$.recipe.ingredients[0].name", "Noodles")])
            .unwrap();
        deltas.push(r.text_delta);
        let r = acc
            .process_partial_args(&[s("$.recipe.ingredients[1].amount", "1 lb")])
            .unwrap();
        deltas.push(r.text_delta);
        let r = acc
            .process_partial_args(&[s("$.recipe.ingredients[1].name", "Beef")])
            .unwrap();
        deltas.push(r.text_delta);
        let r = acc
            .process_partial_args(&[s("$.recipe.name", "Lasagna")])
            .unwrap();
        deltas.push(r.text_delta);
        let r = acc
            .process_partial_args(&[s_continue("$.recipe.steps[0]", "Preheat")])
            .unwrap();
        deltas.push(r.text_delta);
        let r = acc
            .process_partial_args(&[s("$.recipe.steps[0]", " oven.")])
            .unwrap();
        deltas.push(r.text_delta);
        let r = acc
            .process_partial_args(&[s("$.recipe.steps[1]", "Cook.")])
            .unwrap();
        deltas.push(r.text_delta);

        let f = acc.finalize();
        deltas.push(f.closing_delta);
        assert_eq!(deltas.join(""), f.final_json);
    }

    #[test]
    fn will_continue_strings_concat_invariant() {
        let mut acc = GoogleJsonAccumulator::new();
        let mut deltas = Vec::new();

        let r = acc
            .process_partial_args(&[s_continue("$.location", "Bos")])
            .unwrap();
        deltas.push(r.text_delta);
        let r = acc.process_partial_args(&[s("$.location", "ton")]).unwrap();
        deltas.push(r.text_delta);
        let r = acc.process_partial_args(&[n("$.count", 42.0)]).unwrap();
        deltas.push(r.text_delta);

        let f = acc.finalize();
        deltas.push(f.closing_delta);
        assert_eq!(deltas.join(""), f.final_json);
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Provider config  (TS: google-provider.test.ts — config subset)
// ════════════════════════════════════════════════════════════════════════════

mod provider_config_tests {
    use super::*;

    fn test_prompt() -> LanguageModelPrompt {
        vec![LanguageModelPromptMessage {
            role: Role::User,
            content: vec![ContentPart::text("Hello")],
            ..Default::default()
        }]
    }

    fn default_options(prompt: LanguageModelPrompt) -> CallOptions {
        CallOptions::new(prompt)
    }

    async fn mock_ok(server: &MockServer) {
        Mock::given(method("POST"))
            .and(path("/models/gemini-2.0-flash:generateContent"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [{
                    "content": { "parts": [{ "text": "hi" }], "role": "model" },
                    "finishReason": "STOP", "index": 0
                }]
            })))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn default_base_url_is_generativelanguage() {
        let config = GoogleConfig::new("test-api-key");
        assert_eq!(
            config.base_url,
            "https://generativelanguage.googleapis.com/v1beta"
        );
    }

    #[tokio::test]
    async fn custom_base_url_is_used() {
        let server = MockServer::start().await;
        mock_ok(&server).await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("ok");
    }

    #[tokio::test]
    async fn provider_name_is_google() {
        use aimux_core::provider::Provider;
        let config = GoogleConfig::new("test-api-key");
        let provider = GoogleProvider::new(config);
        assert_eq!(provider.name(), "google");
    }

    #[tokio::test]
    async fn model_provider_is_google_generative_ai() {
        let server = MockServer::start().await;
        mock_ok(&server).await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        assert_eq!(model.provider(), "google.generative-ai");
        assert_eq!(model.model_id(), "gemini-2.0-flash");
    }

    #[tokio::test]
    async fn with_base_url_strips_trailing_slash() {
        let config = GoogleConfig::new("key").with_base_url("https://example.com/v1beta/");
        assert_eq!(config.base_url, "https://example.com/v1beta");
    }

    #[tokio::test]
    #[serial]
    async fn from_env_uses_google_generative_ai_api_key() {
        unsafe {
            std::env::set_var("GOOGLE_GENERATIVE_AI_API_KEY", "env-api-key");
        }
        let config = GoogleConfig::from_env().expect("from_env");
        assert_eq!(config.api_key, "env-api-key");
        unsafe {
            std::env::remove_var("GOOGLE_GENERATIVE_AI_API_KEY");
        }
    }

    #[tokio::test]
    #[serial]
    async fn from_env_errors_when_missing() {
        unsafe {
            std::env::remove_var("GOOGLE_GENERATIVE_AI_API_KEY");
        }
        let result = GoogleConfig::from_env();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn api_key_sent_via_x_goog_api_key_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/models/gemini-2.0-flash:generateContent"))
            .and(header("x-goog-api-key", "my-secret-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [{
                    "content": { "parts": [{ "text": "hi" }], "role": "model" },
                    "finishReason": "STOP", "index": 0
                }]
            })))
            .mount(&server)
            .await;

        let config = GoogleConfig::new("my-secret-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        model
            .do_generate(&default_options(test_prompt()))
            .await
            .expect("should succeed — the mock requires the x-goog-api-key header");
    }

    #[tokio::test]
    async fn custom_headers_are_sent_alongside_api_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/models/gemini-2.0-flash:generateContent"))
            .and(header("x-goog-api-key", "test-api-key"))
            .and(header("custom-header", "custom-value"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [{
                    "content": { "parts": [{ "text": "hi" }], "role": "model" },
                    "finishReason": "STOP", "index": 0
                }]
            })))
            .mount(&server)
            .await;

        let config = GoogleConfig::new("test-api-key").with_base_url(server.uri());
        let provider = GoogleProvider::new(config);
        let model = provider.model("gemini-2.0-flash");

        let mut opts = default_options(test_prompt());
        let mut headers = std::collections::HashMap::new();
        headers.insert("Custom-Header".to_string(), "custom-value".to_string());
        opts.headers = Some(headers);

        model
            .do_generate(&opts)
            .await
            .expect("should succeed — mock requires both headers");
    }

    #[tokio::test]
    async fn language_model_trait_method_returns_boxed_model() {
        use aimux_core::provider::Provider;

        let config = GoogleConfig::new("test-api-key");
        let provider = GoogleProvider::new(config);
        let model = provider.language_model("gemini-2.0-flash").unwrap();
        assert_eq!(model.model_id(), "gemini-2.0-flash");
        assert_eq!(model.provider(), "google.generative-ai");
    }
}
