//! Whether a model id is a DeepSeek V4 model (`is-deepseek-v4-model.ts`).

/// Whether a model id is a DeepSeek V4-generation model (thinking mode on by
/// default, `reasoning_content` required on every assistant turn). Only the
/// legacy `deepseek-chat` / `deepseek-reasoner` ids predate V4.
pub(crate) fn is_deepseek_v4_model(model_id: &str) -> bool {
    model_id.contains("deepseek-v4")
        || model_id.starts_with("deepseek-flash")
        || model_id.starts_with("deepseek-pro")
}
