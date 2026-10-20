//! Ports of the upstream `@ai-sdk/mistral` `map-mistral-finish-reason.test.ts`.

use super::*;

mod map_mistral_finish_reason {
    use super::*;

    // The file and its describe block are both named mapMistralFinishReason upstream.
    #[allow(clippy::module_inception)]
    mod map_mistral_finish_reason {
        use super::*;

        /// upstream: @ai-sdk/mistral src/map-mistral-finish-reason.test.ts › mapMistralFinishReason › maps error to error
        #[test]
        fn maps_error_to_error() {
            assert_eq!(
                parse_finish_reason("error").unified,
                FinishReasonUnified::Error
            );
        }

        /// upstream: @ai-sdk/mistral src/map-mistral-finish-reason.test.ts › mapMistralFinishReason › maps unknown finish reasons to other
        #[test]
        fn maps_unknown_finish_reasons_to_other() {
            assert_eq!(
                parse_finish_reason("unknown").unified,
                FinishReasonUnified::Other
            );
        }
    }
}
