//! Ports of `src/convert-to-groq-chat-messages.test.ts` from `@ai-sdk/groq`,
//! checked through the `do_generate` request body.

use super::support::*;

mod user_messages {
    use super::*;

    /// upstream: @ai-sdk/groq src/convert-to-groq-chat-messages.test.ts › user messages › should convert messages with image parts
    #[tokio::test]
    async fn image_parts_from_base64() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let model = make_provider(&server);

        let prompt: LanguageModelPrompt = vec![LanguageModelMessage::User {
            content: vec![
                UserPart::Text(TextPart {
                    text: "Hello".into(),
                    provider_options: None,
                }),
                UserPart::File(FilePart {
                    data: FileData::Data {
                        data: FileBytes::Base64("AAECAw==".into()),
                    },
                    media_type: "image/png".into(),
                    filename: None,
                    provider_options: None,
                }),
            ],
            provider_options: None,
        }];
        model.do_generate(&default_options(prompt)).await.unwrap();

        let body = first_request_body(&server).await;
        let msg = &body["messages"][0];
        assert_eq!(msg["role"], "user");
        assert_eq!(msg["content"][0]["type"], "text");
        assert_eq!(msg["content"][0]["text"], "Hello");
        assert_eq!(msg["content"][1]["type"], "image_url");
        assert_eq!(
            msg["content"][1]["image_url"]["url"],
            "data:image/png;base64,AAECAw=="
        );
    }

    /// upstream: @ai-sdk/groq src/convert-to-groq-chat-messages.test.ts › user messages › should convert messages with only a text part to a string content
    #[tokio::test]
    async fn single_text_becomes_string() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let model = make_provider(&server);

        model
            .do_generate(&default_options(test_prompt()))
            .await
            .unwrap();

        let body = first_request_body(&server).await;
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "Hello");
    }
}

mod tool_calls {
    use super::*;

    /// upstream: @ai-sdk/groq src/convert-to-groq-chat-messages.test.ts › tool calls › should stringify arguments to tool calls
    #[tokio::test]
    async fn tool_call_arguments_stringified() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let model = make_provider(&server);

        let prompt: LanguageModelPrompt = vec![
            LanguageModelMessage::Assistant {
                content: vec![AssistantPart::ToolCall(ToolCallPart {
                    tool_call_id: "quux".into(),
                    tool_name: "thwomp".into(),
                    input: json!({"foo":"bar123"}),
                    provider_executed: None,
                    provider_options: None,
                })],
                provider_options: None,
            },
            LanguageModelMessage::Tool {
                content: vec![ToolPart::ToolResult(ToolResultPart {
                    tool_call_id: "quux".into(),
                    output: ToolResultOutput::Json {
                        value: json!({"oof":"321rab"}),
                        provider_options: None,
                    },
                    tool_name: "thwomp".into(),
                    provider_options: None,
                })],
                provider_options: None,
            },
        ];
        model.do_generate(&default_options(prompt)).await.unwrap();

        let body = first_request_body(&server).await;
        // Assistant message
        assert_eq!(body["messages"][0]["role"], "assistant");
        assert_eq!(body["messages"][0]["content"], "");
        assert_eq!(body["messages"][0]["tool_calls"][0]["id"], "quux");
        assert_eq!(body["messages"][0]["tool_calls"][0]["type"], "function");
        assert_eq!(
            body["messages"][0]["tool_calls"][0]["function"]["name"],
            "thwomp"
        );
        assert_eq!(
            body["messages"][0]["tool_calls"][0]["function"]["arguments"],
            r#"{"foo":"bar123"}"#
        );
        // Tool message
        assert_eq!(body["messages"][1]["role"], "tool");
        assert_eq!(body["messages"][1]["tool_call_id"], "quux");
        assert_eq!(body["messages"][1]["content"], r#"{"oof":"321rab"}"#);
    }

    /// upstream: @ai-sdk/groq src/convert-to-groq-chat-messages.test.ts › tool calls › should send reasoning if present
    #[tokio::test]
    async fn reasoning_in_assistant_message() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let model = make_provider(&server);

        let prompt: LanguageModelPrompt = vec![LanguageModelMessage::Assistant {
            content: vec![
                AssistantPart::Reasoning(ReasoningPart {
                    text: "I think the tool will return the correct value.".into(),
                    provider_options: None,
                }),
                AssistantPart::ToolCall(ToolCallPart {
                    tool_call_id: "quux".into(),
                    tool_name: "thwomp".into(),
                    input: json!({"foo":"bar123"}),
                    provider_executed: None,
                    provider_options: None,
                }),
            ],
            provider_options: None,
        }];
        model.do_generate(&default_options(prompt)).await.unwrap();

        let body = first_request_body(&server).await;
        let msg = &body["messages"][0];
        assert_eq!(msg["role"], "assistant");
        assert_eq!(msg["content"], "");
        assert_eq!(
            msg["reasoning"],
            "I think the tool will return the correct value."
        );
        assert_eq!(msg["tool_calls"][0]["function"]["name"], "thwomp");
    }

    /// upstream: @ai-sdk/groq src/convert-to-groq-chat-messages.test.ts › tool calls › should not include reasoning field when no reasoning content is present
    #[tokio::test]
    async fn no_reasoning_field_when_absent() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let model = make_provider(&server);

        let prompt: LanguageModelPrompt = vec![LanguageModelMessage::Assistant {
            content: vec![AssistantPart::Text(TextPart {
                text: "Hello, how can I help you?".into(),
                provider_options: None,
            })],
            provider_options: None,
        }];
        model.do_generate(&default_options(prompt)).await.unwrap();

        let body = first_request_body(&server).await;
        let msg = &body["messages"][0];
        assert_eq!(msg["role"], "assistant");
        assert_eq!(msg["content"], "Hello, how can I help you?");
        assert!(msg.get("reasoning").is_none() || msg["reasoning"].is_null());
    }

    /// Groq does not support provider file references.
    /// upstream: @ai-sdk/groq src/convert-to-groq-chat-messages.test.ts › tool calls › should throw for file parts with provider references
    #[tokio::test]
    async fn file_reference_unsupported() {
        let server = MockServer::start().await;
        mock_json(&server, text_completion_body()).await;

        let model = make_provider(&server);

        let prompt: LanguageModelPrompt = vec![LanguageModelMessage::User {
            content: vec![UserPart::File(FilePart {
                data: FileData::Reference {
                    reference: [("groq".into(), "file-ref-123".into())].into(),
                },
                media_type: "image/png".into(),
                filename: None,
                provider_options: None,
            })],
            provider_options: None,
        }];
        // upstream convert-to-groq-chat-messages.ts: a provider reference
        // throws UnsupportedFunctionalityError.
        let result = model.do_generate(&default_options(prompt)).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(
            err,
            aimux_core::AiMuxError::UnsupportedFunctionality(_)
        ));
        assert!(
            err.to_string()
                .contains("file parts with provider references"),
            "error should name the unsupported functionality: {err}"
        );
    }
}
