//! Runtime family through the real shared dispatch boundary.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use aimux_ops::dispatch::{self, Ctx};
use aimux_ops::envelope::{OpsError, ProtocolError};
use aimux_ops::handle::{HandleId, Owner};
use serde_json::{Value, json};

fn ctx(owner: Owner) -> Ctx {
    Ctx {
        owner,
        abort: None,
        now: SystemTime::now,
    }
}

#[tokio::test]
async fn description_matches_the_real_manifest_and_digest() {
    let result = dispatch::call(
        ctx(Owner::CProcess),
        HandleId(0),
        "runtime.describe",
        json!({}),
    )
    .await
    .unwrap();
    let manifest = aimux_ops::manifest::manifest();
    assert_eq!(result["version"], env!("CARGO_PKG_VERSION"));
    for field in ["abiVersion", "opsVersion", "manifestRevision"] {
        assert_eq!(result[field], manifest[field]);
    }
    assert_eq!(
        result["manifestDigest"],
        aimux_ops::manifest::manifest_digest()
    );
    assert_eq!(
        result["capabilities"]["handleKinds"],
        manifest["handleKinds"]
    );
    let operations: Vec<_> = manifest["ops"]
        .as_array()
        .unwrap()
        .iter()
        .map(|op| op["name"].clone())
        .collect();
    assert_eq!(result["capabilities"]["operations"], json!(operations));
    assert_eq!(result.as_object().unwrap().len(), 6);
    assert_eq!(result["capabilities"].as_object().unwrap().len(), 2);
}

#[tokio::test]
async fn strict_controls_fail_before_configuration() {
    for (name, params) in [
        ("runtime.describe", json!({"credentials":true})),
        ("runtime.describe", Value::Null),
        ("runtime.configure", json!({"timeoutMs":1})),
        (
            "runtime.configure",
            json!({"logging":{"level":"warn", "body":true}}),
        ),
        ("runtime.configure", json!({"logging":{"level":"verbose"}})),
        ("runtime.configure", json!({"proxy":{"unknown":true}})),
        ("runtime.configure", json!({"proxy":{"http_url":42}})),
        ("runtime.configure", json!({"proxy":[]})),
        ("runtime.configure", json!([])),
        ("runtime.configure", Value::Null),
    ] {
        let error = dispatch::call(ctx(Owner::CProcess), HandleId(0), name, params)
            .await
            .unwrap_err();
        assert!(matches!(error, OpsError::Projected(_)));
        assert!(matches!(
            error.unprojected(),
            OpsError::Protocol(ProtocolError::InvalidParams(_))
        ));
        assert_eq!(error.code(), 301);
    }
}

#[tokio::test]
async fn configuration_summary_and_frozen_proxy_preserve_transport_and_privacy() {
    let before = aimux_provider_utils::fetch::default_fetch();
    let summary = dispatch::call(
        ctx(Owner::CProcess),
        HandleId(0),
        "runtime.configure",
        json!({}),
    )
    .await
    .unwrap();
    assert_eq!(summary["proxy"]["frozen"], true);
    let error = dispatch::call(
        ctx(Owner::CProcess),
        HandleId(0),
        "runtime.configure",
        json!({"proxy":{"http_url":"http://user:secret@localhost:8080"}}),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error.unprojected(),
        OpsError::Protocol(ProtocolError::InvalidState(_))
    ));
    assert_eq!(error.code(), 304);
    assert!(
        !error
            .envelope(SystemTime::now())
            .error
            .to_string()
            .contains("secret")
    );
    assert!(Arc::ptr_eq(
        &before,
        &aimux_provider_utils::fetch::default_fetch()
    ));
}

#[tokio::test]
async fn configuration_error_is_projected_once_by_dispatch() {
    static CLOCK_READS: AtomicUsize = AtomicUsize::new(0);
    fn now() -> SystemTime {
        CLOCK_READS.fetch_add(1, Ordering::SeqCst);
        SystemTime::UNIX_EPOCH + Duration::from_secs(10)
    }
    let mut context = ctx(Owner::CProcess);
    context.now = now;
    let _ = aimux_provider_utils::fetch::default_fetch();
    let error = dispatch::call(
        context,
        HandleId(0),
        "runtime.configure",
        json!({"proxy":{"http_url":"http://different.invalid:9999"}}),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, OpsError::Projected(_)));
    assert_eq!(CLOCK_READS.load(Ordering::SeqCst), 1);
    assert_eq!(
        error.envelope(SystemTime::UNIX_EPOCH),
        error.envelope(SystemTime::now())
    );
    assert_eq!(CLOCK_READS.load(Ordering::SeqCst), 1);
}

#[test]
fn startup_configuration_uses_native_freeze_authorities() {
    const CASE: &str = "AIMUX_RUNTIME_TEST_CASE";
    let Ok(case) = std::env::var(CASE) else {
        for case in ["startup", "native"] {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "startup_configuration_uses_native_freeze_authorities",
                    "--nocapture",
                ])
                .env(CASE, case)
                .env_remove("AIMUX_LOG")
                .env_remove("AIMUX_LOG_LEVEL")
                .status()
                .unwrap();
            assert!(status.success(), "scenario {case}");
        }
        return;
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        if case == "native" {
            aimux_provider_utils::init_logging("info");
        }
        let summary = dispatch::call(
            ctx(Owner::CProcess),
            HandleId(0),
            "runtime.configure",
            json!({}),
        )
        .await
        .unwrap();
        assert_eq!(summary["proxy"]["frozen"], false);
        let logging = dispatch::call(
            ctx(Owner::CProcess),
            HandleId(0),
            "runtime.configure",
            json!({"logging":{"level":"warn"}}),
        )
        .await;
        if case != "startup" {
            assert_eq!(logging.unwrap_err().code(), 304);
        } else {
            assert_eq!(logging.unwrap()["logging"]["requestedLevel"], "warn");
            assert!(
                dispatch::call(
                    ctx(Owner::CProcess),
                    HandleId(0),
                    "runtime.configure",
                    json!({"logging":{"level":"warn"}})
                )
                .await
                .is_ok()
            );
            assert_eq!(
                dispatch::call(
                    ctx(Owner::CProcess),
                    HandleId(0),
                    "runtime.configure",
                    json!({"logging":{"level":"trace"}})
                )
                .await
                .unwrap_err()
                .code(),
                304
            );
        }
        let request = json!({"proxy":{"http_url":"http://user:secret@proxy.invalid:8080"}});
        let summary = dispatch::call(
            ctx(Owner::CProcess),
            HandleId(0),
            "runtime.configure",
            request.clone(),
        )
        .await
        .unwrap();
        assert_eq!(summary["proxy"]["configured"], true);
        assert_eq!(summary["proxy"]["frozen"], false);
        assert!(!summary.to_string().contains("secret"));
        assert!(!summary.to_string().contains("proxy.invalid"));
        let first = aimux_provider_utils::fetch::default_fetch();
        assert!(
            dispatch::call(
                ctx(Owner::CProcess),
                HandleId(0),
                "runtime.configure",
                request
            )
            .await
            .is_ok()
        );
        assert!(Arc::ptr_eq(
            &first,
            &aimux_provider_utils::fetch::default_fetch()
        ));
        assert_eq!(
            dispatch::call(
                ctx(Owner::CProcess),
                HandleId(0),
                "runtime.configure",
                json!({"proxy":{}})
            )
            .await
            .unwrap_err()
            .code(),
            304
        );
    });
}
