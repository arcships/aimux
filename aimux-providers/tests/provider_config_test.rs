use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::sync::{Arc, Mutex};

use aimux_core::error::AiMuxError;
use aimux_core::options::CallOptions;
use aimux_provider_utils::fetch::{Fetch, FetchError, FetchRequest, FetchResponse};
use aimux_provider_utils::{ModalityKind, PackageKind, StringOrEnv};
use aimux_providers::{
    CodexModeSpec, EnvLookupPolicy, ProviderConfigDocument, ProviderConfigSettings,
    ProviderEntrySpec, package_descriptor, package_descriptors, provider_api_key_env,
    provider_default_base_url, provider_from_config, provider_from_config_with_policy,
    provider_from_config_with_transport, provider_names, providers_from_config,
    providers_from_config_with_policy,
};
use futures::future::BoxFuture;
use serde_json::json;
use serial_test::serial;

struct EnvGuard {
    name: &'static str,
    previous: Option<OsString>,
}

impl EnvGuard {
    fn set(name: &'static str, value: Option<&str>) -> Self {
        let previous = std::env::var_os(name);
        // SAFETY: Serialized with `#[serial]` across env-touching tests using unique names.
        unsafe {
            match value {
                Some(v) => std::env::set_var(name, v),
                None => std::env::remove_var(name),
            }
        }
        Self { name, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: Restores previous process env state while serialized.
        unsafe {
            match &self.previous {
                Some(prev) => std::env::set_var(self.name, prev),
                None => std::env::remove_var(self.name),
            }
        }
    }
}

#[derive(Default)]
struct CaptureFetch {
    requests: Mutex<Vec<(String, Option<String>)>>,
}

impl CaptureFetch {
    fn last_request(&self) -> (String, Option<String>) {
        self.requests
            .lock()
            .unwrap()
            .last()
            .expect("captured request")
            .clone()
    }
}

impl Fetch for CaptureFetch {
    fn fetch<'a, 'f>(
        &'a self,
        request: FetchRequest,
    ) -> BoxFuture<'f, Result<FetchResponse, FetchError>>
    where
        'a: 'f,
        Self: 'f,
    {
        let auth = request
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let url = request.url.to_string();
        self.requests.lock().unwrap().push((url, auth));
        Box::pin(async move { Err(FetchError::Other("captured transport".into())) })
    }
}

#[test]
fn package_descriptors_cover_all_packages_in_sorted_order_and_match_helpers() {
    let descriptors = package_descriptors();
    assert_eq!(descriptors.len(), 329);
    assert!(descriptors.windows(2).all(|pair| pair[0].id < pair[1].id));

    let ids: BTreeSet<&str> = descriptors.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids.len(), descriptors.len());
    assert!(ids.contains("openai_compatible"));
    assert!(ids.contains("jev"));
    for name in provider_names() {
        assert!(ids.contains(name), "missing descriptor for {name}");
        let desc = package_descriptor(name).expect("descriptor present");
        assert_eq!(
            desc.api_key_env.as_deref(),
            provider_api_key_env(name),
            "api_key_env parity for {name}"
        );
        assert_eq!(
            desc.default_base_url.as_deref(),
            provider_default_base_url(name),
            "default_base_url parity for {name}"
        );
    }

    let openai = package_descriptor("openai").unwrap();
    assert_eq!(openai.kind, PackageKind::Native);
    assert!(openai.supports_default_instance);
    assert!(openai.supports_discovery);
    assert!(openai.supports_files);
    assert!(!openai.supports_skills);
    assert_eq!(openai.default_method, Some(ModalityKind::LanguageModel));
    assert_eq!(
        openai.named_language_methods,
        vec!["chat", "responses", "completion"]
    );

    let anthropic = package_descriptor("anthropic").unwrap();
    assert!(anthropic.supports_files);
    assert!(anthropic.supports_skills);
    assert_eq!(anthropic.named_language_methods, vec!["messages", "chat"]);

    let compat = package_descriptor("openai_compatible").unwrap();
    assert_eq!(compat.kind, PackageKind::Protocol);
    assert!(!compat.supports_default_instance);
    assert!(
        compat
            .settings
            .iter()
            .any(|s| s.name == "baseUrl" && s.required)
    );
    assert!(
        compat
            .settings
            .iter()
            .any(|s| s.name == "name" && !s.required)
    );

    let jev = package_descriptor("jev").unwrap();
    assert_eq!(jev.kind, PackageKind::Specialized);
    assert!(!jev.supports_default_instance);
    assert_eq!(jev.default_method, Some(ModalityKind::Decision));
    assert_eq!(jev.api_key_env.as_deref(), Some("TYPESAFE_API_KEY"));

    let cloudflare = package_descriptor("cloudflare_workers_ai").unwrap();
    assert_eq!(cloudflare.kind, PackageKind::Preset);
    assert_eq!(cloudflare.template_params.len(), 1);
    assert_eq!(cloudflare.template_params[0].name, "account_id");
    assert!(cloudflare.modalities.contains(&ModalityKind::Decision));

    let cartesia = package_descriptor("cartesia").unwrap();
    assert_eq!(cartesia.default_method, None);
}

#[test]
fn named_language_methods_are_exposed_across_language_vendors() {
    let cases: &[(&str, &str)] = &[
        ("openai", "chat"),
        ("openai", "responses"),
        ("openai", "completion"),
        ("anthropic", "messages"),
        ("anthropic", "chat"),
        ("anthropic_aws", "messages"),
        ("anthropic_aws", "chat"),
        ("azure", "chat"),
        ("azure", "responses"),
        ("codex", "responses"),
        ("xai", "responses"),
        ("huggingface", "responses"),
        ("google", "chat"),
        ("google", "generativeAI"),
        ("google", "generative_ai"),
        ("google_vertex", "chat"),
        ("amazon_bedrock", "chat"),
        ("mistral", "chat"),
        ("groq", "chat"),
        ("deepseek", "chat"),
        ("cohere", "chat"),
        ("ollama", "chat"),
        ("ollama", "completion"),
    ];
    for &(package, method) in cases {
        let provider = aimux_providers::create_provider(package, Default::default()).unwrap();
        let model = provider
            .language_model_named(method, "model-id")
            .unwrap_or_else(|| panic!("{package} missing named method {method}"))
            .unwrap();
        assert_eq!(model.model_id(), "model-id");
        assert!(provider.language_model_named("nonexistent", "m").is_none());
    }

    let compat = provider_from_config(&ProviderEntrySpec {
        package: "openai_compatible".into(),
        settings: ProviderConfigSettings {
            name: Some("custom-compat".into()),
            base_url: Some("https://compat.example/v1".into()),
            ..Default::default()
        },
    })
    .unwrap();
    assert_eq!(
        compat
            .language_model_named("chat", "m")
            .unwrap()
            .unwrap()
            .provider(),
        "custom-compat.chat"
    );
    assert_eq!(
        compat
            .language_model_named("completion", "m")
            .unwrap()
            .unwrap()
            .provider(),
        "custom-compat.completion"
    );

    let elevenlabs = aimux_providers::create_provider("elevenlabs", Default::default()).unwrap();
    assert!(elevenlabs.language_model_named("chat", "m").is_none());
}

#[test]
fn providers_from_config_isolates_multiple_instances_of_same_package() {
    let doc: ProviderConfigDocument = serde_json::from_value(json!({
        "providers": {
            "work": {
                "package": "openai",
                "settings": {
                    "name": "openai-work",
                    "baseUrl": "https://work.example/v1",
                    "apiKey": "sk-work",
                    "organization": "org-work"
                }
            },
            "personal": {
                "package": "openai",
                "settings": {
                    "name": "openai-personal",
                    "baseUrl": "https://personal.example/v1",
                    "apiKey": "sk-personal",
                    "project": "proj-personal"
                }
            },
            "compatDefaultName": {
                "package": "openai_compatible",
                "settings": {
                    "baseUrl": "https://compat.example/v1"
                }
            }
        }
    }))
    .unwrap();

    let providers = providers_from_config(&doc).unwrap();
    assert_eq!(providers.len(), 3);

    let env_doc: ProviderConfigDocument = serde_json::from_value(json!({
        "providers": {
            "blocked": {
                "package": "openai",
                "settings": { "apiKey": { "env": "AIMUX_POLICY_VAR" } }
            }
        }
    }))
    .unwrap();
    assert!(matches!(
        providers_from_config_with_policy(&env_doc, &EnvLookupPolicy::DenyAll),
        Err(AiMuxError::InvalidArgument { .. })
    ));
    assert_eq!(
        providers["work"]
            .language_model("gpt-4o")
            .unwrap()
            .provider(),
        "openai-work.responses"
    );
    assert_eq!(
        providers["personal"]
            .language_model("gpt-4o")
            .unwrap()
            .provider(),
        "openai-personal.responses"
    );
    assert_eq!(
        providers["compatDefaultName"]
            .language_model("m1")
            .unwrap()
            .provider(),
        "compatDefaultName.chat"
    );
}

#[tokio::test]
#[serial]
async fn env_references_respect_policy_and_do_not_parse_env_prefix_strings() {
    let _guard = EnvGuard::set("AIMUX_TEST_CONFIG_KEY", Some("resolved-secret"));
    let _missing = EnvGuard::set("AIMUX_TEST_MISSING_KEY", None);

    let env_spec: ProviderEntrySpec = serde_json::from_value(json!({
        "package": "openai_compatible",
        "settings": {
            "name": "gw",
            "baseUrl": "https://gw.example/v1",
            "apiKey": { "env": "AIMUX_TEST_CONFIG_KEY" }
        }
    }))
    .unwrap();

    assert!(matches!(
        provider_from_config_with_policy(&env_spec, &EnvLookupPolicy::DenyAll),
        Err(AiMuxError::InvalidArgument { .. })
    ));
    assert!(matches!(
        provider_from_config_with_policy(&env_spec, &EnvLookupPolicy::allow_list(["OTHER_KEY"])),
        Err(AiMuxError::InvalidArgument { .. })
    ));

    let capture = Arc::new(CaptureFetch::default());
    let allowed = provider_from_config_with_transport(
        &env_spec,
        &EnvLookupPolicy::allow_list(["AIMUX_TEST_CONFIG_KEY"]),
        Some(capture.clone()),
    )
    .unwrap();
    let _ = allowed
        .language_model("m1")
        .unwrap()
        .do_generate(&CallOptions::new(vec![]))
        .await;
    assert_eq!(
        capture.last_request().1.as_deref(),
        Some("Bearer resolved-secret")
    );

    // Legacy "env:..." string literals are sent verbatim rather than resolving env vars.
    let literal_prefix_spec: ProviderEntrySpec = serde_json::from_value(json!({
        "package": "openai_compatible",
        "settings": {
            "name": "gw",
            "baseUrl": "https://gw.example/v1",
            "apiKey": "env:AIMUX_TEST_CONFIG_KEY"
        }
    }))
    .unwrap();
    let capture_literal = Arc::new(CaptureFetch::default());
    let literal_provider = provider_from_config_with_transport(
        &literal_prefix_spec,
        &EnvLookupPolicy::DenyAll,
        Some(capture_literal.clone()),
    )
    .unwrap();
    let _ = literal_provider
        .language_model("m1")
        .unwrap()
        .do_generate(&CallOptions::new(vec![]))
        .await;
    assert_eq!(
        capture_literal.last_request().1.as_deref(),
        Some("Bearer env:AIMUX_TEST_CONFIG_KEY")
    );

    let missing_spec: ProviderEntrySpec = serde_json::from_value(json!({
        "package": "openai",
        "settings": { "apiKey": { "env": "AIMUX_TEST_MISSING_KEY" } }
    }))
    .unwrap();
    let missing_err = provider_from_config(&missing_spec).err().unwrap();
    assert!(matches!(missing_err, AiMuxError::LoadApiKey { .. }));

    let invalid_env = ProviderEntrySpec {
        package: "openai".into(),
        settings: ProviderConfigSettings {
            api_key: Some(StringOrEnv::env("123_BAD")),
            ..Default::default()
        },
    };
    assert!(matches!(
        provider_from_config(&invalid_env),
        Err(AiMuxError::InvalidArgument { .. })
    ));
}

#[test]
fn supports_specialized_packages_modes_and_strict_validation() {
    let codex_sub = provider_from_config(&ProviderEntrySpec {
        package: "codex".into(),
        settings: ProviderConfigSettings {
            codex_mode: Some(CodexModeSpec::ChatGptAccount {
                token: "tok-123".into(),
                account_id: Some("acct-1".into()),
            }),
            originator: Some("custom-client".into()),
            ..Default::default()
        },
    })
    .unwrap();
    assert_eq!(
        codex_sub
            .language_model("gpt-5.2-codex")
            .unwrap()
            .provider(),
        "codex.responses"
    );

    let codex_conflict = provider_from_config(&ProviderEntrySpec {
        package: "codex".into(),
        settings: ProviderConfigSettings {
            api_key: Some("k".into()),
            codex_mode: Some(CodexModeSpec::ApiKey { api_key: None }),
            ..Default::default()
        },
    });
    assert!(matches!(
        codex_conflict,
        Err(AiMuxError::InvalidArgument { .. })
    ));

    let anthropic_token = provider_from_config(&ProviderEntrySpec {
        package: "anthropic".into(),
        settings: ProviderConfigSettings {
            auth_token: Some("bearer-tok".into()),
            ..Default::default()
        },
    })
    .unwrap();
    assert!(anthropic_token.language_model("claude-sonnet-4-5").is_ok());

    assert!(
        provider_from_config(&ProviderEntrySpec {
            package: "azure".into(),
            settings: ProviderConfigSettings {
                resource_name: Some("my-res".into()),
                api_version: Some("2025-01-01-preview".into()),
                use_deployment_based_urls: Some(true),
                access_token: Some("entra-tok".into()),
                ..Default::default()
            },
        })
        .is_ok()
    );

    assert!(
        provider_from_config(&ProviderEntrySpec {
            package: "amazon_bedrock".into(),
            settings: ProviderConfigSettings {
                region: Some("us-east-1".into()),
                access_key_id: Some("AKIAEXAMPLE".into()),
                secret_access_key: Some("secret-example".into()),
                ..Default::default()
            },
        })
        .is_ok()
    );

    assert!(
        provider_from_config(&ProviderEntrySpec {
            package: "google_vertex".into(),
            settings: ProviderConfigSettings {
                project: Some("my-proj".into()),
                location: Some("us-central1".into()),
                ..Default::default()
            },
        })
        .is_ok()
    );

    let jev = provider_from_config(&ProviderEntrySpec {
        package: "jev".into(),
        settings: ProviderConfigSettings {
            api_key: Some("ts-key".into()),
            endpoint: Some("https://jev.example/v1".into()),
            probability_source: Some("native".into()),
            ..Default::default()
        },
    })
    .unwrap();
    assert!(jev.decision_model("systemone").unwrap().is_ok());

    let cloudflare = provider_from_config(&ProviderEntrySpec {
        package: "cloudflare_workers_ai".into(),
        settings: ProviderConfigSettings {
            params: HashMap::from([("account_id".into(), "acct-123".into())]),
            ..Default::default()
        },
    })
    .unwrap();
    assert!(cloudflare.language_model("@cf/meta/llama").is_ok());

    // Rejects unsupported field per package.
    let bad_field = provider_from_config(&ProviderEntrySpec {
        package: "anthropic".into(),
        settings: ProviderConfigSettings {
            organization: Some("not-allowed".into()),
            ..Default::default()
        },
    });
    assert!(matches!(bad_field, Err(AiMuxError::InvalidArgument { .. })));

    let missing_name = provider_from_config(&ProviderEntrySpec {
        package: "openai_compatible".into(),
        settings: ProviderConfigSettings {
            base_url: Some("https://compat.example/v1".into()),
            ..Default::default()
        },
    });
    assert!(matches!(
        missing_name,
        Err(AiMuxError::InvalidArgument { .. })
    ));

    let empty_key_doc = ProviderConfigDocument {
        providers: [(
            " ".to_string(),
            ProviderEntrySpec {
                package: "openai".into(),
                settings: ProviderConfigSettings::default(),
            },
        )]
        .into_iter()
        .collect(),
    };
    assert!(matches!(
        providers_from_config(&empty_key_doc),
        Err(AiMuxError::InvalidArgument { .. })
    ));

    let unknown_pkg = provider_from_config(&ProviderEntrySpec {
        package: "nonexistent_vendor".into(),
        settings: ProviderConfigSettings::default(),
    });
    assert!(matches!(
        unknown_pkg,
        Err(AiMuxError::NoSuchProvider { .. })
    ));
}

#[test]
fn debug_formatting_redacts_all_secret_literals_and_headers() {
    let doc: ProviderConfigDocument = serde_json::from_value(json!({
        "providers": {
            "secret_test": {
                "package": "codex",
                "settings": {
                    "codexMode": {
                        "type": "chatGptAccount",
                        "token": "top-secret-codex-token",
                        "accountId": "acct-visible"
                    },
                    "headers": {
                        "x-secret-header": "do-not-print-header-value"
                    }
                }
            },
            "anthropic_secret": {
                "package": "anthropic",
                "settings": {
                    "apiKey": "sk-ant-literal-secret",
                    "authToken": "tok-literal-secret"
                }
            }
        }
    }))
    .unwrap();

    let dbg = format!("{doc:?}");
    assert!(!dbg.contains("top-secret-codex-token"));
    assert!(!dbg.contains("do-not-print-header-value"));
    assert!(!dbg.contains("sk-ant-literal-secret"));
    assert!(!dbg.contains("tok-literal-secret"));
    assert!(dbg.contains("[REDACTED]"));
}
