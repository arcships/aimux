//! HTTP header utilities.

use std::collections::HashMap;

/// The SDK version (injected at build time or hardcoded for now).
const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Add a `User-Agent` suffix to the headers.
///
/// Pattern: `ai-sdk/<provider-name>/<version>`
pub fn with_user_agent_suffix(headers: &mut HashMap<String, String>, provider_name: &str) {
    let ua = format!("ai-sdk/{}/{}", provider_name, SDK_VERSION);
    headers.insert("User-Agent".to_string(), ua);
}
