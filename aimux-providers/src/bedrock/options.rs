//! The providerOptions and providerMetadata namespaces of Amazon Bedrock.
//!
//! `@ai-sdk/amazon-bedrock` takes its options from `providerOptions.amazonBedrock`
//! and writes response metadata under `amazonBedrock`. Only that canonical key
//! exists: the SDK's historical `bedrock` alias is not read and not written.
//! This module is the one place that knows the key, so the models never spell
//! it. (Claude-specific `anthropic` options do not
//! travel through the Converse API, so nothing here reads that key.)

use std::collections::HashMap;

use serde_json::{Map, Value};

/// The namespace: read and written.
pub(crate) const AMAZON_BEDROCK: &str = "amazonBedrock";

/// The Bedrock options in a providerOptions container (`amazonBedrock`).
pub(crate) fn read<M: Lookup>(provider_options: Option<&M>) -> Option<&Value> {
    provider_options?.lookup(AMAZON_BEDROCK)
}

/// The Bedrock options in one providerOptions object (a part's, a message's).
pub(crate) fn read_value(provider_options: Option<&Value>) -> Option<&Value> {
    provider_options?.get(AMAZON_BEDROCK)
}

/// Wrap `payload` as response metadata under the namespace key.
pub(crate) fn metadata(payload: Value) -> Value {
    let mut map = Map::new();
    map.insert(AMAZON_BEDROCK.to_string(), payload);
    Value::Object(map)
}

/// A providerOptions container that can be asked for one namespace.
pub(crate) trait Lookup {
    fn lookup(&self, key: &str) -> Option<&Value>;
}

impl Lookup for HashMap<String, Value> {
    fn lookup(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
}

impl Lookup for Map<String, Value> {
    fn lookup(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
}
