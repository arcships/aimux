//! The other `isMistralModel` / `normalizeToolCallId` cases are ported in
//! `aimux-providers/tests/bedrock_remaining_test.rs`.

use crate::bedrock::convert::normalize_tool_call_id;

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
