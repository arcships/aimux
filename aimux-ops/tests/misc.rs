//! Exercise the real misc registrations, owner resolution and projected errors.
use std::sync::Arc;
use std::time::SystemTime;

use aimux_core::provider::Provider;
use aimux_ops::dispatch::{self, Ctx};
use aimux_ops::handle::{Entry, HandleId, HandleKind, HandleRegistry, Owner};
use serde_json::{Value, json};

fn ctx(owner: Owner) -> Ctx {
    Ctx {
        owner,
        abort: None,
        now: SystemTime::now,
    }
}

fn result() -> Value {
    json!({
        "steps": [{
            "callId":"call", "stepNumber":0,
            "model":{"provider":"test", "modelId":"fallback"},
            "content":[{"type":"text", "text":"hello"}],
            "finishReason":"stop", "usage":{}, "performance":aimux_core::step_result::StepResultPerformance::default(), "warnings":[],
            "request":{},
            "response":{"id":"response", "timestamp":"2026-01-01T00:00:00.000Z", "modelId":"", "messages":[]},
            "providerMetadata":{
                "openai":{"logprobs":[{"token":"wrong", "logprob":-1}]},
                "chosen":{"logprobs":[{"token":"right", "logprob":-0.5}]}
            }
        }], "totalUsage":{}
    })
}

#[tokio::test]
async fn output_uses_explicit_metadata_and_model_fallback_without_generation() {
    let owner = Owner::new_node().unwrap();
    let provider = aimux_providers::create_openai(Default::default()).unwrap();
    let model = provider.language_model("fallback").unwrap();
    let registry = HandleRegistry::global();
    let handle = registry.insert(owner, Entry::Language(model)).unwrap();
    for (namespace, expected) in [("chosen", Some("right")), ("missing", None)] {
        let actual = dispatch::call(
            ctx(owner),
            handle,
            "output.toOpenAI",
            json!({
                "result":result(), "metadataNamespace":namespace
            }),
        )
        .await
        .unwrap();
        assert_eq!(actual["model"], "fallback");
        assert_eq!(actual["id"], "response");
        assert_eq!(actual["choices"][0]["message"]["content"], "hello");
        assert_eq!(
            actual["choices"][0]["logprobs"]["content"][0]["token"].as_str(),
            expected
        );
    }
    for request in [
        json!({"result":result()}),
        json!({"result":result(),"metadataNamespace":""}),
        json!({"result":result(),"metadataNamespace":"chosen","extra":true}),
    ] {
        let error = dispatch::call(ctx(owner), handle, "output.toOpenAI", request)
            .await
            .unwrap_err();
        assert_eq!(error.code(), 301);
    }
    assert!(registry.remove(owner, handle).is_some());
}

#[tokio::test]
async fn catalogue_secondary_abort_hides_foreign_owners_and_supports_cancellation() {
    let owner = Owner::new_node().unwrap();
    let foreign = Owner::new_node().unwrap();
    let registry = HandleRegistry::global();
    let abort = aimux_core::AbortSignal::new();
    abort.abort();
    let handle = registry
        .insert(owner, Entry::Abort(Arc::new(abort)))
        .unwrap();
    let params = json!({"sourceUrl":"http://127.0.0.1/catalogue", "abort":handle});
    let foreign_error =
        dispatch::call(ctx(foreign), HandleId(0), "catalogue.fetch", params.clone())
            .await
            .unwrap_err();
    let unknown_error = dispatch::call(
        ctx(foreign),
        HandleId(0),
        "catalogue.fetch",
        json!({
            "sourceUrl":"http://127.0.0.1/catalogue", "abort":HandleId(u64::MAX)
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(foreign_error.code(), 203);
    assert_eq!(
        serde_json::to_value(foreign_error.envelope(SystemTime::UNIX_EPOCH)).unwrap(),
        serde_json::to_value(unknown_error.envelope(SystemTime::UNIX_EPOCH)).unwrap()
    );
    let cancelled = dispatch::call(ctx(owner), HandleId(0), "catalogue.fetch", params)
        .await
        .unwrap_err();
    assert_eq!(cancelled.code(), 13);
    let wrong = registry
        .insert(
            owner,
            Entry::Provider(Arc::new(
                aimux_providers::create_openai(Default::default()).unwrap(),
            )),
        )
        .unwrap();
    let error = dispatch::call(
        ctx(owner),
        HandleId(0),
        "catalogue.fetch",
        json!({"abort":wrong}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), 203);
    assert!(registry.remove(owner, handle).is_some());
    assert!(registry.remove(owner, wrong).is_some());
}

#[tokio::test]
async fn invalid_auth_request_is_projected_once_without_token_values() {
    let owner = Owner::new_node().unwrap();
    let error = dispatch::call(
        ctx(owner),
        HandleId(0),
        "auth.codexRefresh",
        json!({
            "refreshToken":"secret-token", "clientId":"client", "secret-field":true
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), 301);
    let first = error.envelope(SystemTime::UNIX_EPOCH);
    let second = error.envelope(SystemTime::now());
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(second).unwrap()
    );
    assert!(!serde_json::to_string(&first).unwrap().contains("secret"));
}

#[test]
fn misc_metadata_preserves_the_existing_inventory() {
    let names = [
        "catalogue.fetch",
        "toolRepair.context",
        "toolRepair.apply",
        "toolRepair.applyResult",
        "auth.codexRefresh",
        "output.toOpenAI",
    ];
    for name in names {
        let descriptor = dispatch::descriptor(name).unwrap();
        assert_ne!((descriptor.request_type.id)().into_owned(), "AnyValue");
        assert!(!descriptor.source_symbols.is_empty());
        assert_eq!(
            descriptor.allowed_handle_kinds,
            if name == "output.toOpenAI" {
                &[HandleKind::Language][..]
            } else {
                &[]
            }
        );
    }
    assert_eq!(
        dispatch::descriptor("catalogue.fetch")
            .unwrap()
            .runtime_handle_fields,
        &[("abort", &[HandleKind::Abort][..])]
    );
    assert_eq!(
        aimux_ops::manifest::manifest()["ops"]
            .as_array()
            .unwrap()
            .len(),
        72
    );
}
