use crate::anthropic::{AnthropicProviderSettings, create_anthropic};

mod anthropic_provider_custom_provider_name {
    use super::*;

    /// upstream: @ai-sdk/anthropic src/anthropic-provider.test.ts › anthropic provider - custom provider name › should use custom provider name when specified
    #[test]
    fn should_use_custom_provider_name_when_specified() {
        let provider = create_anthropic(AnthropicProviderSettings {
            base_url: Some("https://api.anthropic.com/v1".into()),
            name: Some("my-claude-proxy".into()),
            api_key: Some("test-api-key".into()),
            ..Default::default()
        })
        .unwrap();

        let model = provider.call("claude-3-haiku-20240307");
        assert_eq!(model.provider(), "my-claude-proxy");
    }
}
