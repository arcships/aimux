//! The other `isMistralModel` / `normalizeToolCallId` cases are ported in
//! `aimux-providers/tests/bedrock_remaining_test.rs`.

use crate::bedrock::normalize_tool_call_id::normalize_tool_call_id;

#[allow(clippy::module_inception)] // the describe has the file's name
mod normalize_tool_call_id {
    use super::*;

    /// upstream: @ai-sdk/amazon-bedrock src/normalize-tool-call-id.test.ts › normalizeToolCallId › should keep distinct Bedrock IDs distinct when their prefixes match
    #[test]
    fn should_keep_distinct_bedrock_ids_distinct_when_their_prefixes_match() {
        assert_eq!(
            [
                normalize_tool_call_id("tooluse_Ac1Xq9ZklmNoPq", true),
                normalize_tool_call_id("tooluse_Ac2Yt7WrstUvWx", true),
            ],
            ["7rDVWRig0", "bNZvZKNBZ"]
        );
    }
}

// ── Older ports (previously `tests/bedrock_remaining_test.rs`) ──
//
// Remaining Amazon Bedrock upstream tests, restored from the tool-types branch.
//
// Pinned translation sources (`amazon-bedrock/src/`):
// - `normalize-tool-call-id.test.ts`
// - `convert-amazon-bedrock-usage.test.ts` (raw echo)
// - `amazon-bedrock-provider.test.ts` (factory/auth variants)
// - `convert-to-amazon-bedrock-chat-messages.test.ts` (Mistral IDs)
// - `amazon-bedrock-chat-language-model.test.ts` (request/response cases)

mod bedrock_remaining {
    use crate::bedrock::tests::remaining_support::*;

    use crate::bedrock::normalize_tool_call_id::normalize_tool_call_id;

    mod is_mistral_model {
        use super::*;

        /// upstream: @ai-sdk/amazon-bedrock src/normalize-tool-call-id.test.ts › isMistralModel › should return true for mistral models
        #[test]
        fn is_mistral_model_true_for_mistral() {
            for id in [
                "mistral.mistral-7b-instruct-v0:2",
                "mistral.mixtral-8x7b-instruct-v0:1",
                "mistral.mistral-large-2402-v1:0",
                "mistral.mistral-small-2402-v1:0",
                "mistral.mistral-large-2407-v1:0",
                "mistral.ministral-3-14b-instruct",
                "mistral.ministral-3-8b-instruct",
            ] {
                assert!(model_normalizes_tool_id(id));
            }
        }

        /// upstream: @ai-sdk/amazon-bedrock src/normalize-tool-call-id.test.ts › isMistralModel › should return true for region-prefixed mistral models
        #[test]
        fn is_mistral_model_region_prefixed() {
            assert!(model_normalizes_tool_id(
                "us.mistral.pixtral-large-2502-v1:0"
            ));
            assert!(model_normalizes_tool_id(
                "eu.mistral.mistral-large-2407-v1:0"
            ));
        }

        /// upstream: @ai-sdk/amazon-bedrock src/normalize-tool-call-id.test.ts › isMistralModel › should return false for non-mistral models
        #[test]
        fn is_mistral_model_false_for_non_mistral() {
            for id in [
                "anthropic.claude-3-5-sonnet-20241022-v2:0",
                "amazon.nova-pro-v1:0",
                "openai.gpt-4o",
                "meta.llama3-70b-instruct-v1:0",
            ] {
                assert!(!model_normalizes_tool_id(id));
            }
        }
    }

    mod normalize_tool_call_id {
        use super::*;

        /// upstream: @ai-sdk/amazon-bedrock src/normalize-tool-call-id.test.ts › normalizeToolCallId › should return the original ID when not a Mistral model
        #[test]
        fn normalize_tool_call_id_passthrough_when_not_mistral() {
            let id = "tooluse_bpe71yCfRu2b5i-nKGDr5g";
            assert_eq!(normalize_tool_call_id(id, false), id);
        }

        /// upstream: @ai-sdk/amazon-bedrock src/normalize-tool-call-id.test.ts › normalizeToolCallId › should hash incompatible IDs deterministically for Mistral models
        #[test]
        fn normalize_tool_call_id_hashes_deterministically() {
            let id = "tooluse_bpe71yCfRu2b5i-nKGDr5g";
            assert_eq!(normalize_tool_call_id(id, true), "8eHypBDcw");
            assert_eq!(normalize_tool_call_id(id, true), "8eHypBDcw");
        }

        /// upstream: @ai-sdk/amazon-bedrock src/normalize-tool-call-id.test.ts › normalizeToolCallId › should produce 9 alphanumeric characters for incompatible IDs
        #[test]
        fn normalize_tool_call_id_special_chars() {
            for (id, expected) in [
                ("tool-use_123ABC456", "hvVDqPNyj"),
                ("___abc123DEF___", "TnzPqldGU"),
                ("abc", "GRuIyUwcV"),
                ("12345", "PceAgWDYe"),
                ("___---___", "5C589HVqG"),
            ] {
                let normalized = normalize_tool_call_id(id, true);
                assert_eq!(normalized, expected);
                assert_eq!(normalized.len(), 9);
                assert!(normalized.bytes().all(|c| c.is_ascii_alphanumeric()));
            }
        }

        /// upstream: @ai-sdk/amazon-bedrock src/normalize-tool-call-id.test.ts › normalizeToolCallId › should preserve IDs that are already valid Mistral tool call IDs
        #[test]
        fn normalize_tool_call_id_already_alphanumeric() {
            assert_eq!(normalize_tool_call_id("abcdefghi", true), "abcdefghi");
            assert_eq!(normalize_tool_call_id("abc123XYZ", true), "abc123XYZ");
        }
    }
}
