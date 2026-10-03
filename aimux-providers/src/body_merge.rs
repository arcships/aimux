//! JSON patch merging for request-body overrides.
//!
//! No provider package reads `body_overrides` any more: provider-level body
//! rewrites are a `transform_request_body` closure on the provider settings.
//! The merge helper stays for the tests that exercise deep-merge semantics on
//! built request bodies, and goes with them when those are rewritten.

use serde_json::Value;

/// Apply provider-level `body_overrides` (RFC-0017) to a built request body:
/// deep-merge the JSON patch into `body`; `null` values delete the
/// corresponding key. A no-op when `overrides` is `None`.
///
/// Call it on the finished body, right before sending, so the override can
/// change anything the converter produced.
pub fn apply_body_overrides(body: &mut Value, overrides: Option<&Value>) {
    if let Some(overrides) = overrides {
        deep_merge_json(body, overrides);
    }
}

/// Deep-merge `patch` into `target`: objects merge key by key, `null` removes
/// the key, any other value replaces the target.
pub fn deep_merge_json(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Object(t), Value::Object(p)) => {
            for (k, v) in p {
                match v {
                    Value::Null => {
                        t.remove(k);
                    }
                    _ => {
                        deep_merge_json(t.entry(k).or_insert(Value::Null), v);
                    }
                }
            }
        }
        (target, patch) => *target = patch.clone(),
    }
}
