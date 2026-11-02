//! Tests of `ProviderHeaders` and `transport_user_agent`. Upstream has no
//! shared function or test file for them: each package's `getHeaders` and
//! AWS fetch build the same headers inline.

use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};

use crate::{
    AuthScheme, Credential, HeaderMapOpt, ProviderHeaders, Resolvable, transport_user_agent,
};

fn key(value: &str) -> Credential {
    Credential::Explicit(Resolvable::Value(value.to_owned()))
}

// Not an upstream case: the credential goes on the wire by its scheme, before
// the fixed headers.
#[tokio::test]
async fn puts_the_credential_on_the_wire_by_its_scheme() {
    let fixed = vec![("OpenAI-Organization".to_owned(), "org".to_owned())];
    let bearer = ProviderHeaders::bearer(key("k"), fixed, None)
        .resolve()
        .await
        .unwrap();
    assert_eq!(bearer["Authorization"].as_deref(), Some("Bearer k"));
    assert_eq!(bearer["OpenAI-Organization"].as_deref(), Some("org"));

    let header = ProviderHeaders::new(key("k"), AuthScheme::Header("x-api-key"), Vec::new(), None)
        .resolve()
        .await
        .unwrap();
    assert_eq!(header["x-api-key"].as_deref(), Some("k"));

    let scheme = ProviderHeaders::new(key("k"), AuthScheme::Scheme("Token"), Vec::new(), None)
        .resolve()
        .await
        .unwrap();
    assert_eq!(scheme["Authorization"].as_deref(), Some("Token k"));

    let none = ProviderHeaders::bearer(Credential::None, Vec::new(), None)
        .resolve()
        .await
        .unwrap();
    assert!(none.is_empty());
}

// Not an upstream case: the user's headers override or remove any of them,
// case-insensitively, and the user-agent suffix comes last.
#[tokio::test]
async fn lets_user_headers_override_and_appends_the_user_agent_suffix() {
    let user = HeaderMapOpt::from([
        ("authorization".to_owned(), None),
        ("X-Custom".to_owned(), Some("1".to_owned())),
        ("User-Agent".to_owned(), Some("app/1".to_owned())),
    ]);
    let headers = ProviderHeaders::bearer(key("k"), Vec::new(), Some(user))
        .with_user_agent("openai", "4.0.83")
        .resolve()
        .await
        .unwrap();

    assert!(
        !headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("authorization"))
    );
    assert_eq!(headers["x-custom"].as_deref(), Some("1"));
    assert_eq!(
        headers["user-agent"].as_deref(),
        Some("app/1 ai-sdk-openai/4.0.83")
    );
}

// Not an upstream case: an unset environment variable fails the request for
// `Env` and sends no credential for `OptionalEnv`.
#[tokio::test]
async fn reads_environment_credentials_per_request() {
    let var = "AIMUX_PROVIDER_HEADERS_TEST_UNSET_KEY";
    let env = Credential::Env {
        var: var.to_owned(),
        description: "Test".to_owned(),
    };
    assert!(env.secret().await.is_err());
    assert_eq!(
        Credential::OptionalEnv(var.to_owned())
            .secret()
            .await
            .unwrap(),
        None
    );
}

// Not an upstream case: the last `ai-sdk-provider-utils/<v>` part is renamed
// and the suffix appended; with neither, the header is left as it is.
#[test]
fn rewrites_the_provider_utils_part_and_appends_the_suffix() {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("app/1 ai-sdk-provider-utils/5.0.53"),
    );
    transport_user_agent(
        &mut headers,
        Some("5.0.51"),
        Some("ai-sdk/amazon-bedrock/5.0.100"),
    );
    assert_eq!(
        headers[USER_AGENT],
        "app/1 ai-sdk-provider-utils/5.0.51 ai-sdk/amazon-bedrock/5.0.100"
    );

    let mut untouched = HeaderMap::new();
    untouched.insert(USER_AGENT, HeaderValue::from_static("app/1"));
    transport_user_agent(&mut untouched, Some("5.0.51"), None);
    assert_eq!(untouched[USER_AGENT], "app/1");
}
