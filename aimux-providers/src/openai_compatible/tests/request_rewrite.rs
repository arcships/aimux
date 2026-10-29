//! aimux: the host body rewrites of `request_rewrite.rs` (no upstream test
//! file; upstream's callbacks are tested through the Vertex providers, which
//! `vertex/tests/maas` and `vertex/tests/xai` port).

use std::sync::Arc;

use serde_json::{Value, json};

use crate::openai_compatible::request_rewrite::CompatibleBodyRewrite;
use crate::openai_compatible::{BaseUrl, ChatSettings, CompatModelConfig};
use crate::shared::{AuthScheme, Credential, ProviderHeaders};

fn config(
    body_rewrite: Option<CompatibleBodyRewrite>,
    transform_request_body: Option<crate::openai_compatible::TransformRequestBody>,
) -> CompatModelConfig {
    CompatModelConfig {
        provider: "test.chat".to_string(),
        base_url: BaseUrl::Fixed(String::new()),
        query_params: None,
        headers: ProviderHeaders::new(Credential::None, AuthScheme::Bearer, Vec::new(), None),
        fetch_headers: None,
        fetch: None,
        body_rewrite,
        transform_request_body,
        chat: ChatSettings::default(),
    }
}

#[test]
fn vertex_maas_defaults_max_tokens_only_for_llama_4() {
    let rewrite = CompatibleBodyRewrite::VertexMaas;
    assert_eq!(
        rewrite.apply(json!({"model": "meta/llama-4-scout-17b-16e-instruct-maas"})),
        json!({"model": "meta/llama-4-scout-17b-16e-instruct-maas", "max_tokens": 8192})
    );
    assert_eq!(
        rewrite.apply(
            json!({"model": "meta/llama-4-maverick-17b-128e-instruct-maas", "max_tokens": 64})
        ),
        json!({"model": "meta/llama-4-maverick-17b-128e-instruct-maas", "max_tokens": 64})
    );
    assert_eq!(
        rewrite.apply(json!({"model": "deepseek-ai/deepseek-r1-0528-maas"})),
        json!({"model": "deepseek-ai/deepseek-r1-0528-maas"})
    );
}

#[test]
fn vertex_xai_drops_reasoning_effort() {
    assert_eq!(
        CompatibleBodyRewrite::VertexXai
            .apply(json!({"model": "grok-4", "reasoning_effort": "high"})),
        json!({"model": "grok-4"})
    );
}

#[test]
fn host_rewrite_runs_before_the_user_transform() {
    let user: crate::openai_compatible::TransformRequestBody = Arc::new(|mut body: Value| {
        body["seen_max_tokens"] = body["max_tokens"].clone();
        body
    });
    let body = config(Some(CompatibleBodyRewrite::VertexMaas), Some(user))
        .transform_body(json!({"model": "meta/llama-4-scout-17b-16e-instruct-maas"}));
    assert_eq!(body["seen_max_tokens"], 8192);
    assert_eq!(
        config(None, None).transform_body(json!({"model": "m"})),
        json!({"model": "m"})
    );
}
