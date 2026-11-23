//! Share compiled validators across structured outputs and tool invocations.

use std::sync::{Arc, LazyLock, Mutex};

use indexmap::IndexMap;
use jsonschema::{ValidationError, Validator};
use serde_json::Value;

const MAX_VALIDATORS: usize = 256;
static VALIDATORS: LazyLock<ValidatorCache> = LazyLock::new(|| ValidatorCache::new(MAX_VALIDATORS));

pub(crate) fn validator_for(schema: &Value) -> Result<Arc<Validator>, ValidationError<'static>> {
    VALIDATORS.get_or_compile(schema)
}

struct ValidatorCache {
    capacity: usize,
    entries: Mutex<IndexMap<Vec<u8>, Arc<Validator>>>,
}

impl ValidatorCache {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            entries: Mutex::default(),
        }
    }

    fn get_or_compile(&self, schema: &Value) -> Result<Arc<Validator>, ValidationError<'static>> {
        // Compare the full key and preserve object order. Reordered keywords
        // can affect which validation error is reported first.
        let key = serde_json::to_vec(schema).expect("JSON values serialize to JSON");
        if let Some(validator) = self.entries.lock().unwrap().get(&key) {
            return Ok(Arc::clone(validator));
        }

        // Compilation can be expensive. Let unrelated hits and misses proceed.
        let compiled = Arc::new(jsonschema::validator_for(schema)?);
        let mut entries = self.entries.lock().unwrap();
        if let Some(validator) = entries.get(&key) {
            return Ok(Arc::clone(validator));
        }
        if entries.len() == self.capacity {
            entries.shift_remove_index(0);
        }
        entries.insert(key, Arc::clone(&compiled));
        Ok(compiled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cached_validation_matches_direct_validation_across_eviction() {
        let cache = ValidatorCache::new(2);
        let schema = json!({"type":"object","required":["a"],"additionalProperties":false});
        let instance = json!({"b": 42});
        let direct = jsonschema::validator_for(&schema).unwrap();

        let first = cache.get_or_compile(&schema).unwrap();
        cache.get_or_compile(&json!({"type": "string"})).unwrap();
        cache.get_or_compile(&json!({"type": "number"})).unwrap();

        for validator in [first, cache.get_or_compile(&schema).unwrap()] {
            let expected = direct.validate(&instance).unwrap_err();
            let actual = validator.validate(&instance).unwrap_err();
            assert_eq!(actual.to_string(), expected.to_string());
            assert_eq!(actual.schema_path(), expected.schema_path());
            assert_eq!(actual.instance_path(), expected.instance_path());
        }

        let invalid = json!({"type": 42});
        assert_eq!(
            cache.get_or_compile(&invalid).unwrap_err().to_string(),
            jsonschema::validator_for(&invalid).unwrap_err().to_string()
        );
    }
}
