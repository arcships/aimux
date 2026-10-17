use aimux_providers::provider::ProviderOptions;
use serde_json::json;

#[test]
fn camel_case_provider_config_preserves_the_base_url_and_serializes_canonically() {
    let options: ProviderOptions = serde_json::from_value(json!({
        "baseUrl": "http://127.0.0.1:12345/v1",
        "headers": {"x-test": "1"},
        "params": {"account_id": "account"}
    }))
    .unwrap();
    assert_eq!(
        options.base_url.as_deref(),
        Some("http://127.0.0.1:12345/v1")
    );
    assert_eq!(options.headers.as_ref().unwrap()["x-test"], "1");
    assert_eq!(options.params.as_ref().unwrap()["account_id"], "account");

    let serialized = serde_json::to_value(&options).unwrap();
    assert_eq!(serialized["baseUrl"], "http://127.0.0.1:12345/v1");
    assert!(serialized.get("base_url").is_none());
}

#[test]
fn legacy_base_url_is_read_without_changing_the_canonical_output() {
    let options: ProviderOptions = serde_json::from_value(json!({
        "base_url": "http://127.0.0.1:12345/v1"
    }))
    .unwrap();
    assert_eq!(
        options.base_url.as_deref(),
        Some("http://127.0.0.1:12345/v1")
    );
    assert!(
        serde_json::to_value(options)
            .unwrap()
            .get("baseUrl")
            .is_some()
    );
}

#[test]
fn removed_provider_settings_are_rejected_with_either_spelling() {
    for key in [
        "maxRetries",
        "max_retries",
        "bodyOverrides",
        "body_overrides",
    ] {
        let error = serde_json::from_value::<ProviderOptions>(json!({(key): 0}))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("call-level option") || error.contains("no longer supported"),
            "{key}: {error}"
        );
    }
}
