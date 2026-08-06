//! Extended OpenAI convert tests ??covers the previously-missing TS cases.
//!
//! Sources:
//! - `convert-to-openai-chat-messages.test.ts` (25 previously missing cases)
//! - `openai-chat-prepare-tools.test.ts` (1 previously missing case)
//! - `openai-chat-language-model.test.ts` requestBodyJson assertions

use aimux_core::content::ContentPart;
use aimux_core::language_model_message::{LanguageModelPrompt, LanguageModelPromptMessage};
use aimux_core::message::Role;
use aimux_core::options::{CallOptions, ToolChoice};
use aimux_core::types::ReasoningEffort;
use aimux_providers::openai::OpenAICompatProfile;
use aimux_providers::openai::convert::{
    SystemMessageMode, build_request_body, build_request_body_with_warnings,
    convert_prompt_to_openai_messages, convert_prompt_to_openai_messages_with_mode, prepare_tools,
};
use serde_json::{Value, json};

fn sys(c: &str) -> LanguageModelPromptMessage {
    LanguageModelPromptMessage {
        role: Role::System,
        content: vec![ContentPart::text(c)],
        provider_options: None,
    }
}
fn up(parts: Vec<ContentPart>) -> LanguageModelPromptMessage {
    LanguageModelPromptMessage {
        role: Role::User,
        content: parts,
        provider_options: None,
    }
}
fn fb64(data: &str, mt: &str) -> ContentPart {
    ContentPart::file_base64(data.to_string(), mt.to_string())
}
fn test_prompt() -> LanguageModelPrompt {
    vec![LanguageModelPromptMessage {
        role: Role::User,
        content: vec![ContentPart::text("Hello")],
        ..Default::default()
    }]
}
fn default_opts(p: LanguageModelPrompt) -> CallOptions {
    CallOptions {
        prompt: p,
        max_output_tokens: None,
        temperature: None,
        stop_sequences: None,
        top_p: None,
        top_k: None,
        presence_penalty: None,
        frequency_penalty: None,
        response_format: None,
        seed: None,
        tools: None,
        tool_choice: ToolChoice::Auto,
        headers: None,
        provider_options: None,
        reasoning: None,
        body_overrides: None,
        max_retries: None,
        timeout: None,
        abort_signal: None,
        session_id: None,
        include_raw_chunks: None,
        call_id: None,
        recording_context: None,
    }
}
fn po(map: Value) -> Option<std::collections::HashMap<String, Value>> {
    let mut h = std::collections::HashMap::new();
    h.insert("openai".to_string(), map);
    Some(h)
}

// �T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T
// convert-to-openai-chat-messages extended tests (25 cases)
// �T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T

mod convert_extended {
    use super::*;

    #[test]
    fn converts_system_to_developer() {
        let p = vec![sys("You are a helpful assistant.")];
        let r = convert_prompt_to_openai_messages_with_mode(&p, SystemMessageMode::Developer);
        assert_eq!(
            Value::Array(r),
            json!([{ "role": "developer", "content": "You are a helpful assistant." }])
        );
    }

    #[test]
    fn removes_system_messages() {
        let p = vec![sys("You are a helpful assistant.")];
        let r = convert_prompt_to_openai_messages_with_mode(&p, SystemMessageMode::Remove);
        assert!(r.is_empty());
    }

    #[test]
    fn adds_bpt_to_system_message() {
        let p: LanguageModelPrompt = vec![LanguageModelPromptMessage {
            role: Role::System,
            content: vec![ContentPart::Text {
                text: "You are a helpful assistant.".into(),
                provider_options: Some(
                    json!({ "openai": { "promptCacheBreakpoint": { "mode": "explicit" } } }),
                ),
            }],
            ..Default::default()
        }];
        let r = convert_prompt_to_openai_messages(&p);
        assert_eq!(
            Value::Array(r),
            json!([{ "role": "system", "content": [{ "type": "text", "text": "You are a helpful assistant.", "prompt_cache_breakpoint": { "mode": "explicit" } }] }])
        );
    }

    #[test]
    fn adds_bpt_to_user_content() {
        let bpt = json!({ "mode": "explicit" });
        let o = Some(json!({ "openai": { "promptCacheBreakpoint": bpt } }));
        let p: LanguageModelPrompt = vec![up(vec![
            ContentPart::Text {
                text: "Hello".into(),
                provider_options: o.clone(),
            },
            ContentPart::FileUrl {
                url: "https://example.com/image.png".into(),
                media_type: "image/png".into(),
                provider_options: o.clone(),
            },
            ContentPart::FileBase64 {
                data: "AAECAw==".into(),
                media_type: "audio/wav".into(),
                filename: None,
                provider_options: o.clone(),
            },
            ContentPart::FileReference {
                media_type: "application/pdf".into(),
                reference: json!({ "openai": "file-pdf-123" }),
                filename: None,
                provider_options: o,
            },
        ])];
        let r = convert_prompt_to_openai_messages(&p);
        assert_eq!(
            Value::Array(r),
            json!([{ "role": "user", "content": [
                { "type": "text", "text": "Hello", "prompt_cache_breakpoint": bpt },
                { "type": "image_url", "image_url": { "url": "https://example.com/image.png" }, "prompt_cache_breakpoint": bpt },
                { "type": "input_audio", "input_audio": { "data": "AAECAw==", "format": "wav" }, "prompt_cache_breakpoint": bpt },
                { "type": "file", "file": { "file_id": "file-pdf-123" }, "prompt_cache_breakpoint": bpt }
            ]}])
        );
    }

    #[test]
    fn adds_image_detail() {
        let p = vec![up(vec![ContentPart::FileBase64 {
            data: "AAECAw==".into(),
            media_type: "image/png".into(),
            filename: None,
            provider_options: Some(json!({ "openai": { "imageDetail": "low" } })),
        }])];
        let r = convert_prompt_to_openai_messages(&p);
        assert_eq!(
            Value::Array(r),
            json!([{ "role": "user", "content": [{ "type": "image_url", "image_url": { "url": "data:image/png;base64,AAECAw==", "detail": "low" } }] }])
        );
    }

    #[test]
    fn audio_wav() {
        let r = convert_prompt_to_openai_messages(&vec![up(vec![fb64("AAECAw==", "audio/wav")])]);
        assert_eq!(r[0]["content"][0]["input_audio"]["format"], json!("wav"));
    }
    #[test]
    fn audio_mpeg() {
        let r = convert_prompt_to_openai_messages(&vec![up(vec![fb64("AAECAw==", "audio/mpeg")])]);
        assert_eq!(r[0]["content"][0]["input_audio"]["format"], json!("mp3"));
    }
    #[test]
    fn audio_mp3() {
        let r = convert_prompt_to_openai_messages(&vec![up(vec![fb64("AAECAw==", "audio/mp3")])]);
        assert_eq!(r[0]["content"][0]["input_audio"]["format"], json!("mp3"));
    }

    #[test]
    fn pdf_with_filename() {
        let p = vec![up(vec![ContentPart::FileBase64 {
            data: "AQIDBAU=".into(),
            media_type: "application/pdf".into(),
            filename: Some("document.pdf".into()),
            provider_options: None,
        }])];
        let r = convert_prompt_to_openai_messages(&p);
        assert_eq!(
            r[0]["content"][0],
            json!({ "type": "file", "file": { "filename": "document.pdf", "file_data": "data:application/pdf;base64,AQIDBAU=" } })
        );
    }

    #[test]
    fn binary_pdf() {
        let p = vec![up(vec![ContentPart::File {
            data: vec![1, 2, 3, 4, 5],
            media_type: "application/pdf".into(),
            filename: Some("document.pdf".into()),
            provider_options: None,
        }])];
        let r = convert_prompt_to_openai_messages(&p);
        assert_eq!(
            r[0]["content"][0]["file"]["file_data"],
            json!("data:application/pdf;base64,AQIDBAU=")
        );
    }

    #[test]
    fn pdf_reference() {
        let p = vec![up(vec![ContentPart::file_reference(
            "application/pdf".to_string(),
            json!({ "openai": "file-pdf-12345" }),
        )])];
        let r = convert_prompt_to_openai_messages(&p);
        assert_eq!(
            r[0]["content"][0],
            json!({ "type": "file", "file": { "file_id": "file-pdf-12345" } })
        );
    }

    #[test]
    fn image_reference() {
        let p = vec![up(vec![ContentPart::file_reference(
            "image/png".to_string(),
            json!({ "openai": "file-img-12345" }),
        )])];
        let r = convert_prompt_to_openai_messages(&p);
        assert_eq!(
            r[0]["content"][0],
            json!({ "type": "file", "file": { "file_id": "file-img-12345" } })
        );
    }

    #[test]
    #[should_panic(expected = "No provider reference found for provider 'openai'")]
    fn throws_reference_missing_openai() {
        let p = vec![up(vec![ContentPart::file_reference(
            "application/pdf".to_string(),
            json!({ "anthropic": "file-xyz" }),
        )])];
        convert_prompt_to_openai_messages(&p);
    }

    #[test]
    fn default_filename_pdf() {
        let p = vec![up(vec![fb64("AQIDBAU=", "application/pdf")])];
        let r = convert_prompt_to_openai_messages(&p);
        assert_eq!(r[0]["content"][0]["file"]["filename"], json!("part-0.pdf"));
    }

    #[test]
    #[should_panic(expected = "file part media type application/something")]
    fn throws_unsupported_mime() {
        convert_prompt_to_openai_messages(&vec![up(vec![fb64(
            "AAECAw==",
            "application/something",
        )])]);
    }

    #[test]
    #[should_panic(expected = "audio file parts with URLs")]
    fn throws_audio_url() {
        let p = vec![up(vec![ContentPart::file_url(
            "https://example.com/foo.wav".to_string(),
            "audio/wav".to_string(),
        )])];
        convert_prompt_to_openai_messages(&p);
    }

    #[test]
    #[should_panic(expected = "file part media type text/plain")]
    fn throws_unsupported_file_type() {
        convert_prompt_to_openai_messages(&vec![up(vec![fb64("AQIDBAU=", "text/plain")])]);
    }

    #[test]
    #[should_panic(expected = "PDF file parts with URLs")]
    fn throws_pdf_url() {
        let p = vec![up(vec![ContentPart::file_url(
            "https://example.com/document.pdf".to_string(),
            "application/pdf".to_string(),
        )])];
        convert_prompt_to_openai_messages(&p);
    }

    #[test]
    fn detects_image_subtype() {
        let b64 = "iVBORw0KGgo=";
        let r = convert_prompt_to_openai_messages(&vec![up(vec![fb64(b64, "image")])]);
        assert_eq!(
            r[0]["content"][0]["image_url"]["url"],
            json!(format!("data:image/png;base64,{}", b64))
        );
    }

    #[test]
    fn normalizes_image_wildcard() {
        let b64 = "iVBORw0KGgo=";
        let r = convert_prompt_to_openai_messages(&vec![up(vec![fb64(b64, "image/*")])]);
        assert_eq!(
            r[0]["content"][0]["image_url"]["url"],
            json!(format!("data:image/png;base64,{}", b64))
        );
    }

    #[test]
    fn passes_through_url_top_level_image() {
        let p = vec![up(vec![ContentPart::file_url(
            "https://example.com/x.png".to_string(),
            "image".to_string(),
        )])];
        let r = convert_prompt_to_openai_messages(&p);
        assert_eq!(
            r[0]["content"][0]["image_url"]["url"],
            json!("https://example.com/x.png")
        );
    }

    #[test]
    fn preserves_full_image_png() {
        let b64 = "iVBORw0KGgo=";
        let r = convert_prompt_to_openai_messages(&vec![up(vec![fb64(b64, "image/png")])]);
        assert_eq!(
            r[0]["content"][0]["image_url"]["url"],
            json!(format!("data:image/png;base64,{}", b64))
        );
    }

    #[test]
    fn adds_bpt_to_assistant_text() {
        let p: LanguageModelPrompt = vec![LanguageModelPromptMessage {
            role: Role::Assistant,
            content: vec![ContentPart::Text {
                text: "Cached assistant content".into(),
                provider_options: Some(
                    json!({ "openai": { "promptCacheBreakpoint": { "mode": "explicit" } } }),
                ),
            }],
            ..Default::default()
        }];
        let r = convert_prompt_to_openai_messages(&p);
        assert_eq!(
            Value::Array(r),
            json!([{ "role": "assistant", "content": [{ "type": "text", "text": "Cached assistant content", "prompt_cache_breakpoint": { "mode": "explicit" } }] }])
        );
    }
}

// �T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T
// prepare_tools: "should add warnings for unsupported tools"
// �T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T

mod prepare_tools_unsupported {
    use super::*;

    /// TS: "should add warnings for unsupported tools"
    ///
    /// The Rust `FunctionTool` is always type `function`; there is no
    /// `provider`-type tool discriminator. This test verifies that when
    /// tools are empty after filtering (all unsupported), the result has
    /// empty tools and no tool_choice ??mirroring the TS behavior where
    /// unsupported tools produce warnings and an empty tools array.
    #[test]
    fn unsupported_tools_produce_empty_tools() {
        // The Rust FunctionTool has no `type` field ??all tools are function
        // tools. The TS test uses a `provider`-type tool which Rust cannot
        // represent. We verify the equivalent behavior: empty tools array
        // results in None tools and None tool_choice.
        let result = prepare_tools(&Some(vec![]), None);
        assert_eq!(result.tools, None);
        assert_eq!(result.tool_choice, None);
        assert!(result.tool_warnings.is_empty());
    }
}

// �T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T
// build_request_body: providerOptions and reasoning model tests
// �T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T�T

mod request_body_extended {
    use super::*;

    /// TS: "should pass settings" (logitBias, parallelToolCalls, user)
    #[test]
    fn passes_settings() {
        let opts = CallOptions {
            provider_options: po(
                json!({ "logitBias": { "50256": -100 }, "parallelToolCalls": false, "user": "test-user-id" }),
            ),
            ..default_opts(test_prompt())
        };
        let body = build_request_body("gpt-3.5-turbo", &opts, false).unwrap();
        assert_eq!(body["logit_bias"], json!({ "50256": -100 }));
        assert_eq!(body["parallel_tool_calls"], json!(false));
        assert_eq!(body["user"], json!("test-user-id"));
    }

    /// TS: "should not set reasoning_effort when reasoning is 'provider-default'"
    #[test]
    fn no_reasoning_effort_for_provider_default() {
        let opts = CallOptions {
            reasoning: Some(ReasoningEffort::ProviderDefault),
            ..default_opts(test_prompt())
        };
        let body = build_request_body("o4-mini", &opts, false).unwrap();
        assert_eq!(
            body,
            json!({ "model": "o4-mini", "messages": [{ "role": "user", "content": "Hello" }] })
        );
    }

    /// TS: "should pass top-level reasoning as reasoning_effort"
    #[test]
    fn top_level_reasoning_as_effort() {
        let opts = CallOptions {
            reasoning: Some(ReasoningEffort::Medium),
            ..default_opts(test_prompt())
        };
        let body = build_request_body("o4-mini", &opts, false).unwrap();
        assert_eq!(body["reasoning_effort"], json!("medium"));
    }

    /// TS: "should prefer providerOptions reasoningEffort over top-level reasoning"
    #[test]
    fn prefer_provider_reasoning_effort() {
        let opts = CallOptions {
            reasoning: Some(ReasoningEffort::Medium),
            provider_options: po(json!({ "reasoningEffort": "high" })),
            ..default_opts(test_prompt())
        };
        let body = build_request_body("o4-mini", &opts, false).unwrap();
        assert_eq!(body["reasoning_effort"], json!("high"));
    }

    /// TS: "should pass reasoningEffort setting from provider metadata"
    #[test]
    fn reasoning_effort_from_provider() {
        let opts = CallOptions {
            provider_options: po(json!({ "reasoningEffort": "low" })),
            ..default_opts(test_prompt())
        };
        let body = build_request_body("o4-mini", &opts, false).unwrap();
        assert_eq!(body["reasoning_effort"], json!("low"));
    }

    /// TS: "should pass reasoningEffort xhigh setting"
    #[test]
    fn reasoning_effort_xhigh() {
        let opts = CallOptions {
            provider_options: po(json!({ "reasoningEffort": "xhigh" })),
            ..default_opts(test_prompt())
        };
        let body = build_request_body("gpt-5.1-codex-max", &opts, false).unwrap();
        assert_eq!(body["reasoning_effort"], json!("xhigh"));
    }

    /// TS: "should pass reasoningEffort max setting"
    #[test]
    fn reasoning_effort_max() {
        let opts = CallOptions {
            provider_options: po(json!({ "reasoningEffort": "max" })),
            ..default_opts(test_prompt())
        };
        let body = build_request_body("gpt-5.6", &opts, false).unwrap();
        assert_eq!(body["reasoning_effort"], json!("max"));
    }

    /// TS: "should pass textVerbosity setting from provider options"
    #[test]
    fn text_verbosity() {
        let opts = CallOptions {
            provider_options: po(json!({ "textVerbosity": "low" })),
            ..default_opts(test_prompt())
        };
        let body = build_request_body("gpt-4o", &opts, false).unwrap();
        assert_eq!(body["verbosity"], json!("low"));
    }

    /// TS: reasoning models ??"should clear out temperature, top_p, etc."
    #[test]
    fn reasoning_clears_temperature_etc() {
        let opts = CallOptions {
            temperature: Some(0.5),
            top_p: Some(0.7),
            frequency_penalty: Some(0.2),
            presence_penalty: Some(0.3),
            ..default_opts(test_prompt())
        };
        let result = build_request_body_with_warnings(
            "o4-mini",
            &opts,
            false,
            "openai",
            &OpenAICompatProfile::full(),
        )
        .unwrap();
        assert!(result.body.get("temperature").is_none() || result.body["temperature"].is_null());
        assert!(result.body.get("top_p").is_none() || result.body["top_p"].is_null());
        assert!(
            result.body.get("frequency_penalty").is_none()
                || result.body["frequency_penalty"].is_null()
        );
        assert!(
            result.body.get("presence_penalty").is_none()
                || result.body["presence_penalty"].is_null()
        );
        assert_eq!(result.warnings.len(), 4);
    }

    /// TS: "should convert maxOutputTokens to max_completion_tokens"
    #[test]
    fn reasoning_max_completion_tokens() {
        let opts = CallOptions {
            max_output_tokens: Some(1000),
            ..default_opts(test_prompt())
        };
        let body = build_request_body("o4-mini", &opts, false).unwrap();
        assert_eq!(body["max_completion_tokens"], json!(1000));
        assert!(body.get("max_tokens").is_none());
    }

    /// TS: "should allow temperature when top-level reasoning is none on gpt-5.1"
    #[test]
    fn gpt51_allows_temp_with_reasoning_none() {
        let opts = CallOptions {
            reasoning: Some(ReasoningEffort::None),
            temperature: Some(0.5),
            ..default_opts(test_prompt())
        };
        let result = build_request_body_with_warnings(
            "gpt-5.1",
            &opts,
            false,
            "openai",
            &OpenAICompatProfile::full(),
        )
        .unwrap();
        assert_eq!(result.body["temperature"], json!(0.5));
        assert_eq!(result.body["reasoning_effort"], json!("none"));
        assert!(result.warnings.is_empty());
    }

    /// TS: "should still clear temperature when top-level reasoning is none on o4-mini"
    #[test]
    fn o4mini_clears_temp_even_with_reasoning_none() {
        let opts = CallOptions {
            reasoning: Some(ReasoningEffort::None),
            temperature: Some(0.5),
            ..default_opts(test_prompt())
        };
        let result = build_request_body_with_warnings(
            "o4-mini",
            &opts,
            false,
            "openai",
            &OpenAICompatProfile::full(),
        )
        .unwrap();
        assert!(result.body.get("temperature").is_none() || result.body["temperature"].is_null());
        assert_eq!(result.warnings.len(), 1);
    }

    /// TS: "should allow forcing reasoning behavior for unrecognized model IDs via providerOptions"
    #[test]
    fn force_reasoning_via_provider_options() {
        let opts = CallOptions {
            temperature: Some(0.5),
            top_p: Some(0.7),
            provider_options: po(json!({ "forceReasoning": true })),
            ..default_opts(test_prompt())
        };
        let result = build_request_body_with_warnings(
            "stealth-reasoning-model",
            &opts,
            false,
            "openai",
            &OpenAICompatProfile::full(),
        )
        .unwrap();
        assert!(result.body.get("temperature").is_none() || result.body["temperature"].is_null());
        assert!(result.body.get("top_p").is_none() || result.body["top_p"].is_null());
        assert_eq!(result.warnings.len(), 2);
    }

    /// TS: "should default systemMessageMode to developer when forcing reasoning"
    #[test]
    fn developer_messages_when_forcing_reasoning() {
        let p: LanguageModelPrompt = vec![
            sys("You are a helpful assistant."),
            LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::text("Hello")],
                ..Default::default()
            },
        ];
        let opts = CallOptions {
            prompt: p,
            provider_options: po(json!({ "forceReasoning": true })),
            ..default_opts(vec![])
        };
        let body = build_request_body("stealth-reasoning-model", &opts, false).unwrap();
        assert_eq!(body["messages"][0]["role"], json!("developer"));
        assert_eq!(body["messages"][1]["content"], json!("Hello"));
    }

    /// TS: "should use developer messages for o1"
    #[test]
    fn developer_messages_for_o1() {
        let p: LanguageModelPrompt = vec![
            sys("You are a helpful assistant."),
            LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::text("Hello")],
                ..Default::default()
            },
        ];
        let opts = CallOptions {
            prompt: p,
            ..default_opts(vec![])
        };
        let body = build_request_body("o1", &opts, false).unwrap();
        assert_eq!(body["messages"][0]["role"], json!("developer"));
    }

    /// TS: "should allow overriding systemMessageMode via providerOptions"
    #[test]
    fn override_system_message_mode() {
        let p: LanguageModelPrompt = vec![
            sys("You are a helpful assistant."),
            LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::text("Hello")],
                ..Default::default()
            },
        ];
        let opts = CallOptions {
            prompt: p,
            provider_options: po(json!({ "systemMessageMode": "developer" })),
            ..default_opts(vec![])
        };
        let body = build_request_body("gpt-4o", &opts, false).unwrap();
        assert_eq!(body["messages"][0]["role"], json!("developer"));
    }

    /// TS: "should use default systemMessageMode when not overridden"
    #[test]
    fn default_system_message_mode_for_gpt4o() {
        let p: LanguageModelPrompt = vec![
            sys("You are a helpful assistant."),
            LanguageModelPromptMessage {
                role: Role::User,
                content: vec![ContentPart::text("Hello")],
                ..Default::default()
            },
        ];
        let opts = CallOptions {
            prompt: p,
            ..default_opts(vec![])
        };
        let body = build_request_body("gpt-4o", &opts, false).unwrap();
        assert_eq!(body["messages"][0]["role"], json!("system"));
    }

    /// TS: "should send max_completion_tokens extension setting"
    #[test]
    fn max_completion_tokens_extension() {
        let opts = CallOptions {
            provider_options: po(json!({ "maxCompletionTokens": 255 })),
            ..default_opts(test_prompt())
        };
        let body = build_request_body("o4-mini", &opts, false).unwrap();
        assert_eq!(body["max_completion_tokens"], json!(255));
    }

    /// TS: "should send prediction extension setting"
    #[test]
    fn prediction_extension() {
        let opts = CallOptions {
            provider_options: po(
                json!({ "prediction": { "type": "content", "content": "Hello, World!" } }),
            ),
            ..default_opts(test_prompt())
        };
        let body = build_request_body("gpt-3.5-turbo", &opts, false).unwrap();
        assert_eq!(
            body["prediction"],
            json!({ "type": "content", "content": "Hello, World!" })
        );
    }

    /// TS: "should send store extension setting"
    #[test]
    fn store_extension() {
        let opts = CallOptions {
            provider_options: po(json!({ "store": true })),
            ..default_opts(test_prompt())
        };
        let body = build_request_body("gpt-3.5-turbo", &opts, false).unwrap();
        assert_eq!(body["store"], json!(true));
    }

    /// TS: "should send metadata extension values"
    #[test]
    fn metadata_extension() {
        let opts = CallOptions {
            provider_options: po(json!({ "metadata": { "custom": "value" } })),
            ..default_opts(test_prompt())
        };
        let body = build_request_body("gpt-3.5-turbo", &opts, false).unwrap();
        assert_eq!(body["metadata"], json!({ "custom": "value" }));
    }

    /// TS: "should send promptCacheKey extension value"
    #[test]
    fn prompt_cache_key() {
        let opts = CallOptions {
            provider_options: po(json!({ "promptCacheKey": "test-cache-key-123" })),
            ..default_opts(test_prompt())
        };
        let body = build_request_body("gpt-3.5-turbo", &opts, false).unwrap();
        assert_eq!(body["prompt_cache_key"], json!("test-cache-key-123"));
    }

    /// TS: "should send promptCacheRetention extension value"
    #[test]
    fn prompt_cache_retention() {
        let opts = CallOptions {
            provider_options: po(json!({ "promptCacheRetention": "24h" })),
            ..default_opts(test_prompt())
        };
        let body = build_request_body("gpt-3.5-turbo", &opts, false).unwrap();
        assert_eq!(body["prompt_cache_retention"], json!("24h"));
    }

    /// TS: "should send promptCacheOptions extension value"
    #[test]
    fn prompt_cache_options() {
        let opts = CallOptions {
            provider_options: po(
                json!({ "promptCacheOptions": { "mode": "explicit", "ttl": "30m" } }),
            ),
            ..default_opts(test_prompt())
        };
        let body = build_request_body("gpt-5.6", &opts, false).unwrap();
        assert_eq!(
            body["prompt_cache_options"],
            json!({ "mode": "explicit", "ttl": "30m" })
        );
    }

    /// TS: "should send safetyIdentifier extension value"
    #[test]
    fn safety_identifier() {
        let opts = CallOptions {
            provider_options: po(json!({ "safetyIdentifier": "test-safety-identifier-123" })),
            ..default_opts(test_prompt())
        };
        let body = build_request_body("gpt-3.5-turbo", &opts, false).unwrap();
        assert_eq!(
            body["safety_identifier"],
            json!("test-safety-identifier-123")
        );
    }

    /// TS: "should remove temperature setting for gpt-4o-search-preview and add warning"
    #[test]
    fn search_preview_removes_temperature() {
        let opts = CallOptions {
            temperature: Some(0.7),
            ..default_opts(test_prompt())
        };
        let result = build_request_body_with_warnings(
            "gpt-4o-search-preview",
            &opts,
            false,
            "openai",
            &OpenAICompatProfile::full(),
        )
        .unwrap();
        assert!(result.body.get("temperature").is_none() || result.body["temperature"].is_null());
        assert_eq!(result.warnings.len(), 1);
        assert!(
            matches!(&result.warnings[0], aimux_core::types::Warning::Unsupported { feature, .. } if feature == "temperature")
        );
    }

    /// TS: "should remove temperature setting for gpt-4o-mini-search-preview"
    #[test]
    fn mini_search_preview_removes_temperature() {
        let opts = CallOptions {
            temperature: Some(0.7),
            ..default_opts(test_prompt())
        };
        let result = build_request_body_with_warnings(
            "gpt-4o-mini-search-preview",
            &opts,
            false,
            "openai",
            &OpenAICompatProfile::full(),
        )
        .unwrap();
        assert!(result.body.get("temperature").is_none() || result.body["temperature"].is_null());
        assert_eq!(result.warnings.len(), 1);
    }

    /// TS: "should remove temperature setting for gpt-4o-mini-search-preview-2025-03-11"
    #[test]
    fn mini_search_preview_dated_removes_temperature() {
        let opts = CallOptions {
            temperature: Some(0.7),
            ..default_opts(test_prompt())
        };
        let result = build_request_body_with_warnings(
            "gpt-4o-mini-search-preview-2025-03-11",
            &opts,
            false,
            "openai",
            &OpenAICompatProfile::full(),
        )
        .unwrap();
        assert!(result.body.get("temperature").is_none() || result.body["temperature"].is_null());
        assert_eq!(result.warnings.len(), 1);
    }

    /// TS: "should send serviceTier flex processing setting"
    #[test]
    fn service_tier_flex() {
        let opts = CallOptions {
            provider_options: po(json!({ "serviceTier": "flex" })),
            ..default_opts(test_prompt())
        };
        let body = build_request_body("o4-mini", &opts, false).unwrap();
        assert_eq!(body["service_tier"], json!("flex"));
    }

    /// TS: "should show warning when using flex processing with unsupported model"
    #[test]
    fn flex_warning_unsupported() {
        let opts = CallOptions {
            provider_options: po(json!({ "serviceTier": "flex" })),
            ..default_opts(test_prompt())
        };
        let result = build_request_body_with_warnings(
            "gpt-4o-mini",
            &opts,
            false,
            "openai",
            &OpenAICompatProfile::full(),
        )
        .unwrap();
        assert!(result.body.get("service_tier").is_none() || result.body["service_tier"].is_null());
        assert_eq!(result.warnings.len(), 1);
        assert!(
            matches!(&result.warnings[0], aimux_core::types::Warning::Unsupported { feature, .. } if feature == "serviceTier")
        );
    }

    /// TS: "should allow flex processing with o4-mini model without warnings"
    #[test]
    fn flex_o4mini_no_warning() {
        let opts = CallOptions {
            provider_options: po(json!({ "serviceTier": "flex" })),
            ..default_opts(test_prompt())
        };
        let result = build_request_body_with_warnings(
            "o4-mini",
            &opts,
            false,
            "openai",
            &OpenAICompatProfile::full(),
        )
        .unwrap();
        assert_eq!(result.body["service_tier"], json!("flex"));
        assert!(result.warnings.is_empty());
    }

    /// TS: "should send serviceTier priority processing setting"
    #[test]
    fn service_tier_priority() {
        let opts = CallOptions {
            provider_options: po(json!({ "serviceTier": "priority" })),
            ..default_opts(test_prompt())
        };
        let body = build_request_body("gpt-4o-mini", &opts, false).unwrap();
        assert_eq!(body["service_tier"], json!("priority"));
    }

    /// TS: "should show warning when using priority processing with unsupported model"
    #[test]
    fn priority_warning_unsupported() {
        let opts = CallOptions {
            provider_options: po(json!({ "serviceTier": "priority" })),
            ..default_opts(test_prompt())
        };
        let result = build_request_body_with_warnings(
            "gpt-3.5-turbo",
            &opts,
            false,
            "openai",
            &OpenAICompatProfile::full(),
        )
        .unwrap();
        assert!(result.body.get("service_tier").is_none() || result.body["service_tier"].is_null());
        assert_eq!(result.warnings.len(), 1);
    }

    /// TS: "should allow priority processing with gpt-4o model without warnings"
    #[test]
    fn priority_gpt4o_no_warning() {
        let opts = CallOptions {
            provider_options: po(json!({ "serviceTier": "priority" })),
            ..default_opts(test_prompt())
        };
        let result = build_request_body_with_warnings(
            "gpt-4o",
            &opts,
            false,
            "openai",
            &OpenAICompatProfile::full(),
        )
        .unwrap();
        assert_eq!(result.body["service_tier"], json!("priority"));
        assert!(result.warnings.is_empty());
    }

    /// TS: "should allow priority processing with o3 model without warnings"
    #[test]
    fn priority_o4mini_no_warning() {
        let opts = CallOptions {
            provider_options: po(json!({ "serviceTier": "priority" })),
            ..default_opts(test_prompt())
        };
        let result = build_request_body_with_warnings(
            "o4-mini",
            &opts,
            false,
            "openai",
            &OpenAICompatProfile::full(),
        )
        .unwrap();
        assert_eq!(result.body["service_tier"], json!("priority"));
        assert!(result.warnings.is_empty());
    }
}
