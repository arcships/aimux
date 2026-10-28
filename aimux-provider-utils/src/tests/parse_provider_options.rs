//! Tests of `parse_provider_options`. Upstream has no
//! `parse-provider-options.test.ts`; these cover the behaviour of
//! `parse-provider-options.ts` that its callers rely on.

use aimux_core::AiMuxError;
use aimux_core::shared::SharedProviderOptions;
use serde::Deserialize;
use serde_json::json;

use crate::parse_provider_options;

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Options {
    user_id: Option<String>,
    top_logprobs: Option<u8>,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictOptions {
    mode: Option<String>,
}

fn provider_options(value: serde_json::Value) -> SharedProviderOptions {
    serde_json::from_value(value).expect("valid provider options")
}

// Not an upstream case: `providerOptions?.[provider] == null` returns undefined.
#[test]
fn returns_none_without_the_namespace() {
    let options = provider_options(json!({ "other": { "userId": "u" } }));

    assert_eq!(
        parse_provider_options::<Options>("test", Some(&options)).unwrap(),
        None
    );
    assert_eq!(
        parse_provider_options::<Options>("test", None).unwrap(),
        None
    );
}

// Not an upstream case: a zod object drops keys it does not declare.
#[test]
fn parses_the_namespace_and_drops_unknown_keys() {
    let options = provider_options(json!({
        "test": { "userId": "u", "topLogprobs": 3, "extra": true },
    }));

    assert_eq!(
        parse_provider_options::<Options>("test", Some(&options)).unwrap(),
        Some(Options {
            user_id: Some("u".into()),
            top_logprobs: Some(3),
        })
    );
}

// Not an upstream case: a failed schema throws `InvalidArgumentError` with
// message `invalid ${provider} provider options` and the validation error
// as its cause.
#[test]
fn reports_a_mismatch_as_invalid_provider_options_with_a_cause() {
    let options = provider_options(json!({ "test": { "topLogprobs": "three" } }));

    let error = parse_provider_options::<Options>("test", Some(&options)).unwrap_err();

    let AiMuxError::InvalidArgument { message, cause } = error else {
        panic!("expected InvalidArgument, got {error:?}");
    };
    assert_eq!(
        message,
        "Invalid argument for parameter providerOptions: invalid test provider options"
    );
    assert!(cause.is_some());
}

// Not an upstream case: a strict schema (`z.strictObject`) rejects unknown keys.
#[test]
fn a_strict_schema_rejects_unknown_keys() {
    let options = provider_options(json!({ "test": { "mode": "a", "extra": 1 } }));

    assert!(parse_provider_options::<StrictOptions>("test", Some(&options)).is_err());
}

#[derive(Debug, PartialEq, Deserialize)]
struct OptionalFields {
    #[serde(default, deserialize_with = "crate::optional_field")]
    user: Option<String>,
}

// Not an upstream case: a zod `.optional()` field may be absent, but an
// explicit `null` fails the schema as a wrong type does.
#[test]
fn optional_field_is_none_when_absent_and_rejects_null() {
    let absent = provider_options(json!({ "test": {} }));
    assert_eq!(
        parse_provider_options::<OptionalFields>("test", Some(&absent)).unwrap(),
        Some(OptionalFields { user: None })
    );

    let null = provider_options(json!({ "test": { "user": null } }));
    let error = parse_provider_options::<OptionalFields>("test", Some(&null)).unwrap_err();
    assert!(
        matches!(&error, AiMuxError::InvalidArgument { message, cause: Some(_) }
            if message == "Invalid argument for parameter providerOptions: invalid test provider options"),
        "{error:?}"
    );
}
