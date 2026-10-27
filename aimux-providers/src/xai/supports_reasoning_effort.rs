//! `supports-reasoning-effort.ts`: which Grok models accept `reasoning_effort`.

/// Models that reject the `reasoning_effort` parameter.
/// Matches `^grok-4\.20(-\d{4})?-(non-)?reasoning$`.
fn is_model_without_reasoning_effort(model_id: &str) -> bool {
    let rest = match model_id.strip_prefix("grok-4.20") {
        Some(r) => r,
        None => return false,
    };
    let rest = if rest.len() >= 5
        && rest.starts_with('-')
        && rest[1..5].chars().all(|c| c.is_ascii_digit())
    {
        &rest[5..]
    } else {
        rest
    };
    rest == "-reasoning" || rest == "-non-reasoning"
}

/// Whether the model accepts the `reasoning_effort` parameter.
#[must_use]
pub fn supports_reasoning_effort(model_id: &str) -> bool {
    !is_model_without_reasoning_effort(model_id)
}
