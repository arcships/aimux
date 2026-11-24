//! Process runtime discovery and startup configuration.

use crate::envelope::ProtocolError;
use crate::handle::HandleKind;
use crate::manifest::{MANIFEST_REVISION, manifest_digest};
use aimux_provider_utils::{ProxyConfig, fetch, logging};
use serde::{Deserialize, Deserializer, Serialize};

use crate::dispatch::{
    CallFuture, Ctx, Handler, Params, Target, decode as parse, serialize_result,
};
use crate::op::{
    ABI_VERSION, Cancellation, OPS_VERSION, OpDescriptor, OpMode, RequestBytePolicy, Scope, TypeRef,
};

pub(crate) static OPS: &[OpDescriptor] = &[
    OpDescriptor {
        name: "runtime.describe",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[],
        request_type: TypeRef::of::<RuntimeDescribeParams>(),
        result_type: TypeRef::of::<serde_json::Value>(),
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        side_effects: false,
        retry_owner: "none",
        source_symbols: &[
            "aimux_ops::manifest::manifest",
            "aimux_ops::manifest::manifest_digest",
        ],
    },
    OpDescriptor {
        name: "runtime.configure",
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: &[],
        request_type: TypeRef::of::<RuntimeConfigureParams>(),
        result_type: TypeRef::of::<serde_json::Value>(),
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[],
        required_capabilities: &[],
        cancellation: Cancellation::None,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        side_effects: true,
        retry_owner: "none",
        source_symbols: &[
            "aimux_provider_utils::logging::configure_logging",
            "aimux_provider_utils::fetch::configure_startup_proxy",
        ],
    },
];

pub(crate) fn register(r: &mut Vec<(&'static OpDescriptor, Handler)>) {
    r.push((&OPS[0], Handler::Call(describe)));
    r.push((&OPS[1], Handler::Call(configure)));
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RuntimeDescribeParams {}

#[derive(Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct RuntimeConfigureParams {
    logging: Option<LoggingConfig>,
    #[serde(default, deserialize_with = "deserialize_proxy")]
    #[schemars(with = "Option<super::handles::TransportProxy>")]
    proxy: Option<ProxyConfig>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct LoggingConfig {
    level: LogLevel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
enum LogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

fn deserialize_proxy<'de, D>(deserializer: D) -> Result<Option<ProxyConfig>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<super::handles::ProxySettings>::deserialize(deserializer)
        .map(|settings| settings.map(|settings| settings.0))
}

fn describe(_ctx: Ctx, _target: Target, params: Params<'_>) -> CallFuture<'_> {
    Box::pin(async move {
        let _: RuntimeDescribeParams = parse(params)?;
        serialize_result(description())
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Description {
    version: &'static str,
    abi_version: u32,
    ops_version: &'static str,
    manifest_revision: &'static str,
    manifest_digest: String,
    capabilities: RuntimeCapabilities,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeCapabilities {
    operations: Vec<&'static str>,
    handle_kinds: &'static [HandleKind],
    packages: Vec<&'static str>,
}

fn description() -> Description {
    Description {
        version: env!("CARGO_PKG_VERSION"),
        abi_version: ABI_VERSION,
        ops_version: OPS_VERSION,
        manifest_revision: MANIFEST_REVISION,
        manifest_digest: manifest_digest(),
        capabilities: RuntimeCapabilities {
            operations: crate::dispatch::registrations()
                .iter()
                .map(|(descriptor, _)| descriptor.name)
                .collect(),
            handle_kinds: HandleKind::ALL,
            packages: crate::ops::provider::packages(),
        },
    }
}

fn configure(_ctx: Ctx, _target: Target, params: Params<'_>) -> CallFuture<'_> {
    Box::pin(async move {
        let params: RuntimeConfigureParams = parse(params)?;
        if let Some(config) = params.logging {
            logging::configure_logging(config.level.as_str())
                .map_err(|_| ProtocolError::InvalidState("logging is frozen".into()))?;
        }
        if let Some(proxy) = params.proxy {
            fetch::configure_startup_proxy(proxy)
                .map_err(|_| ProtocolError::InvalidState("startup proxy is frozen".into()))?;
        }
        let (configured, frozen) = fetch::startup_proxy_state();
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Logging {
            requested_level: Option<&'static str>,
            frozen: bool,
        }

        #[derive(Serialize)]
        struct Proxy {
            configured: bool,
            frozen: bool,
        }

        #[derive(Serialize)]
        struct Configuration {
            logging: Logging,
            proxy: Proxy,
        }

        serialize_result(Configuration {
            logging: Logging {
                requested_level: logging::requested_logging_level(),
                frozen: logging::logging_initialized(),
            },
            proxy: Proxy { configured, frozen },
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::{OpsError, ProtocolError};
    use serde_json::{Value, json};

    #[test]
    fn controls_reject_unknown_keys_and_wrong_types() {
        for params in [
            json!({"timeoutMs": 10}),
            json!({"logging": {"level": "warn", "body": true}}),
            json!({"logging": {"level": "verbose"}}),
            json!({"proxy": {"http_url": "http://localhost", "unknown": true}}),
            json!({"proxy": {"http_url": 42}}),
            json!({"proxy": []}),
            json!([]),
            Value::Null,
        ] {
            assert!(matches!(
                parse::<RuntimeConfigureParams>(params),
                Err(OpsError::Protocol(ProtocolError::InvalidParams { .. }))
            ));
        }
        assert!(parse::<RuntimeDescribeParams>(json!({"credentials": true})).is_err());
        assert!(parse::<RuntimeDescribeParams>(json!({})).is_ok());
    }

    #[test]
    fn proxy_reuses_existing_wire_type() {
        let params: RuntimeConfigureParams = parse(json!({"proxy": {
            "http_url": "http://user:secret@localhost:8080",
            "https_url": null,
            "no_proxy": "localhost",
        }}))
        .unwrap();
        let proxy = params.proxy.unwrap();
        assert_eq!(
            proxy.http_url.as_deref(),
            Some("http://user:secret@localhost:8080")
        );
        assert_eq!(proxy.no_proxy.as_deref(), Some("localhost"));
    }

    #[test]
    fn description_uses_manifest_capabilities_and_digest() {
        let manifest = crate::manifest::manifest();
        let expected = json!({
            "version": env!("CARGO_PKG_VERSION"),
            "abiVersion": ABI_VERSION,
            "opsVersion": OPS_VERSION,
            "manifestRevision": manifest["manifestRevision"],
            "manifestDigest": manifest["digest"],
            "capabilities": {
                "operations": manifest["ops"].as_array().unwrap().iter()
                    .map(|op| &op["name"]).collect::<Vec<_>>(),
                "handleKinds": manifest["handleKinds"],
                "packages": manifest["packages"],
            },
        });
        assert_eq!(
            serialize_result(description()).unwrap(),
            expected.to_string()
        );
    }

    #[test]
    fn descriptors_are_global_calls_with_accurate_side_effects() {
        assert_eq!(OPS.len(), 2);
        for op in OPS {
            assert!(op.allowed_handle_kinds.is_empty());
            assert!(matches!(op.mode, OpMode::Call));
            assert_eq!(op.cancellation, Cancellation::None);
            assert_eq!(op.scope, Scope::Current);
            assert!(op.runtime_handle_fields.is_empty());
        }
        assert!(!OPS[0].side_effects);
        assert!(OPS[1].side_effects);
        let mut handlers = Vec::new();
        register(&mut handlers);
        assert_eq!(handlers.len(), 2);
    }
}
