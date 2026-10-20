//! Port of `src/chat/is-deepseek-v4-model.test.ts` (`@ai-sdk/deepseek` 3.0.56).

use crate::deepseek::is_v4_model::is_deepseek_v4_model;

mod is_deep_seek_v4_model {
    use super::*;

    /// upstream: @ai-sdk/deepseek src/chat/is-deepseek-v4-model.test.ts › isDeepSeekV4Model › should treat %s as V4
    #[test]
    fn should_treat_as_v4() {
        for model_id in [
            "deepseek-v4-pro",
            "deepseek-v4-pro-0813",
            "deepseek-v4-flash",
            "deepseek-v4-flash-0731",
            "deepseek-v4-flash-vision-exp",
            "deepseek-flash",
            "deepseek-pro",
        ] {
            assert!(is_deepseek_v4_model(model_id), "{model_id} should be V4");
        }
    }

    /// upstream: @ai-sdk/deepseek src/chat/is-deepseek-v4-model.test.ts › isDeepSeekV4Model › should not treat legacy %s as V4
    #[test]
    fn should_not_treat_legacy_as_v4() {
        for model_id in ["deepseek-chat", "deepseek-reasoner"] {
            assert!(
                !is_deepseek_v4_model(model_id),
                "{model_id} should not be V4"
            );
        }
    }
}
