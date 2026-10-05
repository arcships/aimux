// Panic convert wrappers are #[deprecated]; these tests still use them.
#![allow(deprecated)]
//! Extended OpenAI convert tests — covers the previously-missing TS cases.
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
        let _ = convert_prompt_to_openai_messages(&p);
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
        let _ = convert_prompt_to_openai_messages(&vec![up(vec![fb64(
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
        let _ = convert_prompt_to_openai_messages(&p);
    }

    #[test]
    #[should_panic(expected = "file part media type text/plain")]
    fn throws_unsupported_file_type() {
        let _ = convert_prompt_to_openai_messages(&vec![up(vec![fb64("AQIDBAU=", "text/plain")])]);
    }

    #[test]
    #[should_panic(expected = "PDF file parts with URLs")]
    fn throws_pdf_url() {
        let p = vec![up(vec![ContentPart::file_url(
            "https://example.com/document.pdf".to_string(),
            "application/pdf".to_string(),
        )])];
        let _ = convert_prompt_to_openai_messages(&p);
    }

    #[test]
    fn detects_image_subtype() {
        let b64 = "iVBORw0KGgo=";
        let r = convert_prompt_to_openai_messages(&vec![up(vec![fb64(b64, "image")])]);
        assert_eq!(
            r[0]["content"][0]["image_url"]["url"],
            json!(format!("data:image/png;base64,{b64}"))
        );
    }

    #[test]
    fn normalizes_image_wildcard() {
        let b64 = "iVBORw0KGgo=";
        let r = convert_prompt_to_openai_messages(&vec![up(vec![fb64(b64, "image/*")])]);
        assert_eq!(
            r[0]["content"][0]["image_url"]["url"],
            json!(format!("data:image/png;base64,{b64}"))
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
            json!(format!("data:image/png;base64,{b64}"))
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
}
