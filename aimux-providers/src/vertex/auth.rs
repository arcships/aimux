//! Application default Google credentials and OAuth access-token generation.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use aimux_core::error::AiMuxError;
use aimux_provider_utils::{
    FetchFunction, HttpBody, HttpRequest, Resolvable, create_binary_response_handler,
    create_json_response_handler, get_from_api, post_to_api,
};
use base64::Engine;
use ring::{
    rand::SystemRandom,
    signature::{RSA_PKCS1_SHA256, RsaKeyPair},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const CLOUD_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

/// OAuth scopes, as a space-delimited string or an array.
#[derive(Clone, Debug, serde::Serialize, Deserialize)]
#[serde(untagged)]
pub enum GoogleAuthScopes {
    String(String),
    Array(Vec<String>),
}

impl GoogleAuthScopes {
    fn array(&self) -> Vec<String> {
        match self {
            Self::String(value) if value.is_empty() => Vec::new(),
            Self::String(value) => vec![value.clone()],
            Self::Array(values) => values.clone(),
        }
    }

    fn joined(&self) -> String {
        match self {
            Self::String(value) => value.clone(),
            Self::Array(values) => values.join(" "),
        }
    }
}

/// Options passed to the Google credential resolver.
#[derive(Clone, Default)]
pub struct GoogleAuthOptions {
    /// Inline service-account or authorized-user credentials.
    pub credentials: Option<Value>,
    /// Credential JSON filename (`keyFile` in the upstream library).
    pub key_file: Option<PathBuf>,
    /// Alias of `key_file` (`keyFilename` upstream).
    pub key_filename: Option<PathBuf>,
    /// OAuth scopes. Defaults to the Cloud Platform scope.
    pub scopes: Option<GoogleAuthScopes>,
    /// Project associated with the credentials, independent of the API endpoint project.
    pub project_id: Option<String>,
    /// Overrides the access-token client.
    pub auth_client: Option<Resolvable<Option<String>>>,
    /// Options passed to the OAuth client. Unsupported options fail explicitly.
    pub client_options: Option<Value>,
    /// Service domain of the Google Cloud universe.
    pub universe_domain: Option<String>,
    /// Google auth-library API-key mode (not a Vertex Express-mode key).
    pub api_key: Option<String>,
}

impl std::fmt::Debug for GoogleAuthOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoogleAuthOptions")
            .field("credentials", &self.credentials.is_some())
            .field("key_file", &self.key_file)
            .field("key_filename", &self.key_filename)
            .field("scopes", &self.scopes)
            .field("project_id", &self.project_id)
            .field("auth_client", &self.auth_client.is_some())
            .field("client_options", &self.client_options.is_some())
            .field("universe_domain", &self.universe_domain)
            .field("api_key", &self.api_key.is_some())
            .finish()
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: Option<u64>,
    refresh_token: Option<String>,
}

struct CachedToken {
    token: String,
    expiry: Option<Instant>,
    credentials: Option<Value>,
}

pub(super) struct GoogleAuth {
    options: GoogleAuthOptions,
    token: Mutex<Option<CachedToken>>,
    fetch: Option<FetchFunction>,
}

impl GoogleAuth {
    pub(super) fn new(options: GoogleAuthOptions, fetch: Option<FetchFunction>) -> Self {
        Self {
            options,
            token: Mutex::new(None),
            fetch,
        }
    }

    fn credentials(&self) -> Result<Option<Value>, AiMuxError> {
        if let Some(credentials) = &self.options.credentials {
            return Ok(Some(credentials.clone()));
        }
        let explicit = self
            .options
            .key_filename
            .clone()
            .filter(|path| !path.as_os_str().is_empty())
            .or_else(|| {
                self.options
                    .key_file
                    .clone()
                    .filter(|path| !path.as_os_str().is_empty())
            })
            .or_else(|| {
                std::env::var_os("GOOGLE_APPLICATION_CREDENTIALS")
                    .filter(|value| !value.is_empty())
                    .or_else(|| {
                        std::env::var_os("google_application_credentials")
                            .filter(|value| !value.is_empty())
                    })
                    .map(PathBuf::from)
            });
        let path = explicit.or_else(|| {
            let directory = std::env::var_os("CLOUDSDK_CONFIG")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .or_else(|| {
                    if cfg!(windows) {
                        std::env::var_os("APPDATA")
                            .filter(|value| !value.is_empty())
                            .map(|directory| PathBuf::from(directory).join("gcloud"))
                    } else {
                        std::env::var_os("HOME")
                            .filter(|value| !value.is_empty())
                            .map(|home| PathBuf::from(home).join(".config/gcloud"))
                    }
                })?;
            let path = directory.join("application_default_credentials.json");
            path.exists().then_some(path)
        });
        let Some(path) = path else {
            return Ok(None);
        };
        let data = std::fs::read(path).map_err(|error| {
            AiMuxError::InvalidArgument(format!(
                "Could not read Google application credentials: {error}"
            ))
        })?;
        serde_json::from_slice(&data).map(Some).map_err(|error| {
            AiMuxError::InvalidArgument(format!("Invalid Google application credentials: {error}"))
        })
    }

    pub(super) async fn access_token(&self) -> Result<String, AiMuxError> {
        if let Some(client) = &self.options.auth_client {
            return Ok(client.resolve().await?.unwrap_or_else(|| "null".into()));
        }
        self.validate_options()?;
        let mut cache = self.token.lock().await;
        if let Some(cached) = cache.as_ref()
            && cached.expiry.is_none_or(|expiry| expiry > Instant::now())
        {
            return Ok(cached.token.clone());
        }
        let mut credentials = match cache.as_ref() {
            Some(cached) => cached.credentials.clone(),
            None => self.credentials()?,
        };
        if cache.is_none()
            && let Some(credential) = credentials.as_ref()
            && credential.get("type").and_then(Value::as_str) == Some("authorized_user")
        {
            for field in ["client_id", "client_secret", "refresh_token"] {
                required(credential, field)?;
            }
            if let Some(initial) = self
                .options
                .client_options
                .as_ref()
                .and_then(|options| options.get("credentials"))
                && let Some(token) = initial
                    .get("access_token")
                    .and_then(Value::as_str)
                    .filter(|token| !token.is_empty())
            {
                let threshold = self
                    .options
                    .client_options
                    .as_ref()
                    .and_then(|options| options.get("eagerRefreshThresholdMillis"))
                    .and_then(Value::as_i64)
                    .filter(|value| *value != 0)
                    .unwrap_or(300_000);
                let remaining = initial
                    .get("expiry_date")
                    .and_then(Value::as_i64)
                    .map(|expiry| {
                        expiry
                            .saturating_sub(chrono::Utc::now().timestamp_millis())
                            .saturating_sub(threshold)
                    });
                if remaining.is_none_or(|remaining| remaining > 0) {
                    let expiry = remaining
                        .map(|remaining| {
                            Instant::now()
                                .checked_add(Duration::from_millis(remaining as u64))
                                .ok_or_else(|| {
                                    AiMuxError::InvalidResponseData(
                                        "Google OAuth token expiry is out of range".into(),
                                    )
                                })
                        })
                        .transpose()?;
                    let token = token.to_string();
                    *cache = Some(CachedToken {
                        token: token.clone(),
                        expiry,
                        credentials,
                    });
                    return Ok(token);
                }
            }
        }
        let response = self.credential_token(credentials.clone()).await?;
        if let Some(token) = &response.refresh_token
            && let Some(credentials) = credentials.as_mut()
            && credentials.get("type").and_then(Value::as_str)
                == Some("external_account_authorized_user")
        {
            credentials["refresh_token"] = json!(token);
        }
        let external = credentials
            .as_ref()
            .and_then(|value| value.get("type"))
            .and_then(Value::as_str)
            .is_some_and(|kind| {
                matches!(
                    kind,
                    "external_account" | "external_account_authorized_user"
                )
            });
        if !external && response.access_token.is_empty() {
            return Err(AiMuxError::InvalidResponseData(
                "Could not refresh Google access token".into(),
            ));
        }
        let threshold = self
            .options
            .client_options
            .as_ref()
            .and_then(|options| options.get("eagerRefreshThresholdMillis"))
            .and_then(Value::as_u64)
            .filter(|threshold| external || *threshold != 0)
            .unwrap_or(300_000);
        let expiry = response
            .expires_in
            .map(|seconds| {
                Instant::now()
                    .checked_add(
                        Duration::from_secs(seconds)
                            .saturating_sub(Duration::from_millis(threshold)),
                    )
                    .ok_or_else(|| {
                        AiMuxError::InvalidResponseData(
                            "Google OAuth token expiry is out of range".into(),
                        )
                    })
            })
            .transpose()?;
        *cache = Some(CachedToken {
            token: response.access_token.clone(),
            expiry,
            credentials,
        });
        Ok(response.access_token)
    }

    fn credential_token(
        &self,
        credentials: Option<Value>,
    ) -> futures::future::BoxFuture<'_, Result<TokenResponse, AiMuxError>> {
        Box::pin(async move {
            Ok(match credentials {
                Some(credentials)
                    if credentials.get("type").and_then(Value::as_str)
                        == Some("authorized_user") =>
                {
                    self.exchange(
                        vec![
                            ("grant_type", "refresh_token".into()),
                            ("client_id", required(&credentials, "client_id")?.into()),
                            (
                                "client_secret",
                                required(&credentials, "client_secret")?.into(),
                            ),
                            (
                                "refresh_token",
                                required(&credentials, "refresh_token")?.into(),
                            ),
                        ],
                        TOKEN_URL,
                    )
                    .await?
                }
                Some(credentials)
                    if credentials
                        .get("type")
                        .and_then(Value::as_str)
                        .is_none_or(|kind| kind == "service_account") =>
                {
                    let email = required(&credentials, "client_email")?;
                    let key = required(&credentials, "private_key")?;
                    let audience = TOKEN_URL;
                    let scopes = self
                        .options
                        .scopes
                        .as_ref()
                        .map_or_else(|| CLOUD_SCOPE.to_string(), GoogleAuthScopes::joined);
                    let timestamp = chrono::Utc::now().timestamp();
                    let claims = json!({"iss": email, "scope": scopes, "aud": audience, "iat": timestamp, "exp": timestamp + 3600});
                    let mut header = json!({"alg": "RS256", "typ": "JWT"});
                    if let Some(id) = credentials.get("private_key_id").and_then(Value::as_str) {
                        header["kid"] = json!(id);
                    }
                    let encoding = &base64::engine::general_purpose::URL_SAFE_NO_PAD;
                    let unsigned = format!(
                        "{}.{}",
                        encoding.encode(header.to_string()),
                        encoding.encode(claims.to_string())
                    );
                    let der = base64::engine::general_purpose::STANDARD
                        .decode(
                            key.lines()
                                .filter(|line| !line.starts_with("-----"))
                                .collect::<String>(),
                        )
                        .map_err(|_| {
                            AiMuxError::InvalidArgument(
                                "Invalid Google service-account private key encoding".into(),
                            )
                        })?;
                    let key_pair = RsaKeyPair::from_pkcs8(&der)
                        .or_else(|_| RsaKeyPair::from_der(&der))
                        .map_err(|_| {
                            AiMuxError::InvalidArgument(
                                "Invalid Google service-account RSA private key".into(),
                            )
                        })?;
                    let mut signature = vec![0; key_pair.public().modulus_len()];
                    key_pair
                        .sign(
                            &RSA_PKCS1_SHA256,
                            &SystemRandom::new(),
                            unsigned.as_bytes(),
                            &mut signature,
                        )
                        .map_err(|_| {
                            AiMuxError::InvalidArgument(
                                "Could not sign Google service-account assertion".into(),
                            )
                        })?;
                    let assertion = format!("{unsigned}.{}", encoding.encode(signature));
                    self.exchange(
                        vec![
                            (
                                "grant_type",
                                "urn:ietf:params:oauth:grant-type:jwt-bearer".into(),
                            ),
                            ("assertion", assertion),
                        ],
                        audience,
                    )
                    .await?
                }
                Some(credentials)
                    if credentials.get("type").and_then(Value::as_str)
                        == Some("external_account_authorized_user") =>
                {
                    self.exchange_headers(
                        vec![
                            ("grant_type", "refresh_token".into()),
                            (
                                "refresh_token",
                                required(&credentials, "refresh_token")?.into(),
                            ),
                        ],
                        credentials
                            .get("token_url")
                            .and_then(Value::as_str)
                            .unwrap_or(&format!(
                                "https://sts.{}/v1/oauthtoken",
                                self.universe(&credentials)
                            )),
                        basic_auth(&credentials)?,
                    )
                    .await?
                }
                Some(credentials)
                    if credentials.get("type").and_then(Value::as_str)
                        == Some("external_account") =>
                {
                    self.external_token(&normalize_external_credentials(credentials))
                        .await?
                }
                Some(credentials)
                    if credentials.get("type").and_then(Value::as_str)
                        == Some("impersonated_service_account") =>
                {
                    let source = credentials.get("source_credentials").ok_or_else(|| {
                        AiMuxError::InvalidArgument(
                            "Google impersonation credentials require source_credentials".into(),
                        )
                    })?;
                    let token = self.credential_token(Some(source.clone())).await?;
                    self.impersonate(
                        &credentials,
                        required(&credentials, "service_account_impersonation_url")?,
                        &token.access_token,
                    )
                    .await?
                }
                Some(credentials)
                    if credentials.get("type").and_then(Value::as_str)
                        == Some("gdch_service_account") =>
                {
                    self.gdch_token(&credentials).await?
                }
                Some(credentials) => {
                    return Err(AiMuxError::UnsupportedFunctionality(format!(
                        "Google credential type '{}' is not supported",
                        credentials
                            .get("type")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                    )));
                }
                None => {
                    let host = std::env::var("GCE_METADATA_IP")
                        .ok()
                        .filter(|value| !value.is_empty())
                        .or_else(|| {
                            std::env::var("GCE_METADATA_HOST")
                                .ok()
                                .filter(|value| !value.is_empty())
                        })
                        .unwrap_or_else(|| "http://169.254.169.254".into());
                    let host = if host.starts_with("http://") || host.starts_with("https://") {
                        host
                    } else {
                        format!("http://{host}")
                    };
                    let mut url = url::Url::parse(&host).map_err(|error| {
                        AiMuxError::InvalidArgument(format!(
                            "Invalid Google metadata host: {error}"
                        ))
                    })?;
                    url.set_path("/computeMetadata/v1/instance/service-accounts/default/token");
                    url.set_query(None);
                    let scopes = self
                        .options
                        .scopes
                        .as_ref()
                        .map_or_else(|| vec![CLOUD_SCOPE.into()], GoogleAuthScopes::array)
                        .join(",");
                    if !scopes.is_empty() {
                        url.query_pairs_mut().append_pair("scopes", &scopes);
                    }
                    let response = get_from_api(
                        HttpRequest {
                            url: url.into(),
                            headers: vec![("Metadata-Flavor".into(), "Google".into())],
                            response_timeout: Some(Duration::from_secs(3)),
                            fetch: self.fetch.clone(),
                            ..Default::default()
                        },
                        create_json_response_handler::<TokenResponse>(),
                        crate::google::google_failed_response_handler(),
                    )
                    .await?;
                    if !response.response_headers.iter().any(|(name, value)| {
                        name.eq_ignore_ascii_case("Metadata-Flavor") && value == "Google"
                    }) {
                        return Err(AiMuxError::InvalidResponseData(
                            "Invalid Google metadata response Metadata-Flavor header".into(),
                        ));
                    }
                    response.value
                }
            })
        })
    }

    async fn gdch_token(&self, credentials: &Value) -> Result<TokenResponse, AiMuxError> {
        if required(credentials, "format_version")? != "1" {
            return Err(AiMuxError::InvalidArgument(
                "Google GDCH credentials require format_version 1".into(),
            ));
        }
        let client = self.options.client_options.as_ref();
        let audience = client.and_then(|options| options.get("apiAudience")).and_then(Value::as_str).filter(|value| !value.is_empty()).ok_or_else(|| AiMuxError::InvalidArgument("Audience cannot be null or empty for GDCH service account credentials; provide client_options.apiAudience".into()))?;
        let fetch = match (
            self.fetch.clone(),
            credentials
                .get("ca_cert_path")
                .and_then(Value::as_str)
                .filter(|path| !path.is_empty()),
        ) {
            (None, Some(path)) => Some(super::auth_certificate::ca_fetch(path)?),
            (fetch, _) => fetch,
        };
        let token_url = required(credentials, "token_uri")?;
        let identity = format!(
            "system:serviceaccount:{}:{}",
            required(credentials, "project")?,
            required(credentials, "name")?
        );
        let now = chrono::Utc::now().timestamp();
        let lifetime = client
            .and_then(|options| options.get("lifetime"))
            .and_then(Value::as_i64)
            .filter(|value| *value != 0)
            .unwrap_or(3600);
        let expiry = now.checked_add(lifetime).ok_or_else(|| {
            AiMuxError::InvalidArgument("Invalid Google GDCH token lifetime".into())
        })?;
        let header =
            json!({"alg": "ES256", "typ": "JWT", "kid": required(credentials, "private_key_id")?});
        let claims =
            json!({"iss": identity, "sub": identity, "iat": now, "exp": expiry, "aud": token_url});
        let encoding = &base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let unsigned = format!(
            "{}.{}",
            encoding.encode(header.to_string()),
            encoding.encode(claims.to_string())
        );
        let key = required(credentials, "private_key")?;
        let mut der = base64::engine::general_purpose::STANDARD
            .decode(
                key.lines()
                    .filter(|line| !line.starts_with("-----"))
                    .collect::<String>(),
            )
            .map_err(|_| {
                AiMuxError::InvalidArgument("Invalid Google GDCH private key encoding".into())
            })?;
        if key.contains("BEGIN EC PRIVATE KEY") {
            let mut wrapped = vec![
                0x02, 0x01, 0x00, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01,
                0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07,
            ];
            wrapped.extend(der_wrap(0x04, der));
            der = der_wrap(0x30, wrapped);
        }
        let random = SystemRandom::new();
        let pair = ring::signature::EcdsaKeyPair::from_pkcs8(
            &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
            &der,
            &random,
        )
        .map_err(|_| AiMuxError::InvalidArgument("Invalid Google GDCH P-256 private key".into()))?;
        let signature = pair.sign(&random, unsigned.as_bytes()).map_err(|_| {
            AiMuxError::InvalidArgument("Could not sign Google GDCH assertion".into())
        })?;
        let assertion = format!("{unsigned}.{}", encoding.encode(signature.as_ref()));
        let response = post_to_api(HttpRequest { url: token_url.into(), response_timeout: Some(Duration::from_secs(10)), fetch, ..Default::default() }, HttpBody::Json(json!({"audience": audience, "grant_type": "urn:ietf:params:oauth:token-type:token-exchange", "requested_token_type": "urn:ietf:params:oauth:token-type:access_token", "subject_token": assertion, "subject_token_type": "urn:k8s:params:oauth:token-type:serviceaccount"})), create_json_response_handler::<TokenResponse>(), crate::google::google_failed_response_handler()).await?.value;
        if response.access_token.is_empty()
            || response.expires_in.is_none_or(|seconds| seconds == 0)
        {
            return Err(AiMuxError::InvalidResponseData(
                "Google GDCH token response requires access_token and expires_in".into(),
            ));
        }
        Ok(response)
    }

    async fn external_token(&self, credentials: &Value) -> Result<TokenResponse, AiMuxError> {
        let source = credentials.get("credential_source").ok_or_else(|| {
            AiMuxError::InvalidArgument(
                "Google external credentials require credential_source".into(),
            )
        })?;
        let file = source
            .get("file")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty());
        let url = source
            .get("url")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty());
        let certificate = source.get("certificate").filter(|value| !value.is_null());
        if (file.is_some() && url.is_some())
            || (url.is_some() && certificate.is_some())
            || (file.is_some() && certificate.is_some())
        {
            return Err(AiMuxError::InvalidArgument(
                "Google identity pool credentials require exactly one of file, URL or certificate"
                    .into(),
            ));
        }
        let parse_format = source.get("environment_id").is_none()
            && source.get("executable").is_none()
            && certificate.is_none();
        let mut fetch = self.fetch.clone();
        let token = if let Some(certificate) = certificate {
            let (token, certificate_fetch) =
                super::auth_certificate::certificate(certificate, fetch)?;
            fetch = certificate_fetch;
            token
        } else if source.get("environment_id").is_some() {
            super::auth_external::aws(credentials, self.fetch.clone()).await?
        } else if source.get("executable").is_some() {
            super::auth_external::executable(credentials).await?
        } else if let Some(file) = file {
            std::fs::read_to_string(file).map_err(|error| {
                AiMuxError::InvalidArgument(format!("Could not read Google subject token: {error}"))
            })?
        } else if let Some(url) = url {
            let headers = source
                .get("headers")
                .and_then(Value::as_object)
                .map(|values| {
                    values
                        .iter()
                        .map(|(name, value)| {
                            value
                                .as_str()
                                .map(|value| (name.clone(), value.to_string()))
                                .ok_or_else(|| {
                                    AiMuxError::InvalidArgument(
                                        "Google credential_source headers must be strings".into(),
                                    )
                                })
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?
                .unwrap_or_default();
            let bytes = get_from_api(
                HttpRequest {
                    url: url.into(),
                    headers,
                    fetch: self.fetch.clone(),
                    ..Default::default()
                },
                create_binary_response_handler(),
                crate::google::google_failed_response_handler(),
            )
            .await?
            .value;
            String::from_utf8(bytes.to_vec()).map_err(|_| {
                AiMuxError::InvalidResponseData("Google subject token is not UTF-8".into())
            })?
        } else {
            return Err(AiMuxError::UnsupportedFunctionality(
                "Google external credential sources other than file or URL are not supported"
                    .into(),
            ));
        };
        let token = if !parse_format {
            token
        } else {
            match source
                .get("format")
                .and_then(|format| format.get("type"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .unwrap_or("text")
            {
                "text" => token,
                "json" => {
                    let value: Value = serde_json::from_str(&token).map_err(|error| {
                        AiMuxError::InvalidResponseData(format!(
                            "Invalid Google subject-token JSON: {error}"
                        ))
                    })?;
                    required(
                        &value,
                        required(&source["format"], "subject_token_field_name")?,
                    )?
                    .into()
                }
                format => {
                    return Err(AiMuxError::UnsupportedFunctionality(format!(
                        "Google subject-token format {format} is not supported"
                    )));
                }
            }
        };
        if token.is_empty() {
            return Err(AiMuxError::InvalidResponseData(
                "Unable to parse Google subject token".into(),
            ));
        }
        let token_auth = GoogleAuth::new(self.options.clone(), fetch);
        let scope = self
            .options
            .scopes
            .as_ref()
            .map_or_else(|| CLOUD_SCOPE.into(), GoogleAuthScopes::joined);
        let mut fields = vec![
            (
                "grant_type",
                "urn:ietf:params:oauth:grant-type:token-exchange".into(),
            ),
            (
                "requested_token_type",
                "urn:ietf:params:oauth:token-type:access_token".into(),
            ),
            ("audience", required(credentials, "audience")?.into()),
            (
                "subject_token_type",
                required(credentials, "subject_token_type")?.into(),
            ),
            ("subject_token", token),
            (
                "scope",
                if credentials
                    .get("service_account_impersonation_url")
                    .is_some()
                {
                    CLOUD_SCOPE.into()
                } else {
                    scope
                },
            ),
        ];
        if let Some(project) = credentials
            .get("workforce_pool_user_project")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            let audience = required(credentials, "audience")?;
            if !regex::Regex::new(
                r"//iam\.googleapis\.com/locations/[^/]+/workforcePools/[^/]+/providers/.+",
            )
            .expect("static workforce audience pattern")
            .is_match(audience)
            {
                return Err(AiMuxError::InvalidArgument(
                    "workforce_pool_user_project must not be set for non-workforce credentials"
                        .into(),
                ));
            }
            if credentials
                .get("client_id")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            {
                fields.push(("options", json!({"userProject": project}).to_string()));
            }
        }
        let response = token_auth
            .exchange_headers(
                fields,
                credentials
                    .get("token_url")
                    .and_then(Value::as_str)
                    .unwrap_or(&format!(
                        "https://sts.{}/v1/token",
                        self.universe(credentials)
                    )),
                basic_auth(credentials)?,
            )
            .await?;
        if let Some(url) = credentials
            .get("service_account_impersonation_url")
            .and_then(Value::as_str)
        {
            return token_auth
                .impersonate(credentials, url, &response.access_token)
                .await;
        }
        Ok(response)
    }

    async fn impersonate(
        &self,
        credentials: &Value,
        url: &str,
        token: &str,
    ) -> Result<TokenResponse, AiMuxError> {
        #[derive(Deserialize)]
        struct ImpersonationToken {
            #[serde(rename = "accessToken")]
            access_token: String,
            #[serde(rename = "expireTime")]
            expire_time: String,
        }
        let scopes = self
            .options
            .scopes
            .as_ref()
            .map_or_else(|| vec![CLOUD_SCOPE.into()], GoogleAuthScopes::array);
        let lifetime = credentials
            .get("lifetime")
            .or_else(|| {
                credentials
                    .get("service_account_impersonation")
                    .and_then(|options| options.get("token_lifetime_seconds"))
            })
            .and_then(Value::as_u64)
            .unwrap_or(3600);
        let impersonated_url;
        let url = if credentials.get("type").and_then(Value::as_str)
            == Some("impersonated_service_account")
        {
            if url.len() > 256 {
                return Err(AiMuxError::InvalidArgument(
                    "Google impersonation target principal is too long".into(),
                ));
            }
            let target = url
                .rsplit('/')
                .next()
                .and_then(|target| {
                    target
                        .strip_suffix(":generateAccessToken")
                        .or_else(|| target.strip_suffix(":generateIdToken"))
                })
                .filter(|target| !target.is_empty())
                .ok_or_else(|| {
                    AiMuxError::InvalidArgument(
                        "Invalid Google impersonation target principal".into(),
                    )
                })?;
            let endpoint = credentials
                .get("endpoint")
                .and_then(Value::as_str)
                .unwrap_or("https://iamcredentials.googleapis.com");
            impersonated_url =
                format!("{endpoint}/v1/projects/-/serviceAccounts/{target}:generateAccessToken");
            &impersonated_url
        } else {
            url
        };
        let mut body = json!({"scope": scopes, "lifetime": format!("{lifetime}s")});
        if credentials.get("type").and_then(Value::as_str) == Some("impersonated_service_account") {
            body["delegates"] = credentials.get("delegates").cloned().unwrap_or(json!([]));
        }
        let response = post_to_api(
            HttpRequest {
                url: url.into(),
                headers: vec![("Authorization".into(), format!("Bearer {token}"))],
                fetch: self.fetch.clone(),
                ..Default::default()
            },
            HttpBody::Json(body),
            create_json_response_handler::<ImpersonationToken>(),
            crate::google::google_failed_response_handler(),
        )
        .await?
        .value;
        let expiry =
            chrono::DateTime::parse_from_rfc3339(&response.expire_time).map_err(|error| {
                AiMuxError::InvalidResponseData(format!(
                    "Invalid Google impersonation expiry: {error}"
                ))
            })?;
        Ok(TokenResponse {
            access_token: response.access_token,
            expires_in: Some((expiry.timestamp() - chrono::Utc::now().timestamp()).max(0) as u64),
            refresh_token: None,
        })
    }

    fn universe<'a>(&'a self, credentials: &'a Value) -> &'a str {
        self.options
            .universe_domain
            .as_deref()
            .or_else(|| {
                self.options
                    .client_options
                    .as_ref()
                    .and_then(|options| {
                        options
                            .get("universe_domain")
                            .or_else(|| options.get("universeDomain"))
                    })
                    .and_then(Value::as_str)
            })
            .or_else(|| credentials.get("universe_domain").and_then(Value::as_str))
            .unwrap_or("googleapis.com")
    }

    fn validate_options(&self) -> Result<(), AiMuxError> {
        let client = self.options.client_options.as_ref();
        if let Some(options) = client {
            let Some(options) = options.as_object() else {
                return Err(AiMuxError::InvalidArgument(
                    "Google auth client_options must be an object".into(),
                ));
            };
            for (name, value) in options {
                match name.as_str() {
                    "projectId" | "project_id" | "quotaProjectId" | "quota_project_id"
                    | "universeDomain" | "universe_domain" | "apiKey" => {
                        if !value.is_string() && !value.is_null() {
                            return Err(AiMuxError::InvalidArgument(format!(
                                "Google auth client option {name} must be a string"
                            )));
                        }
                    }
                    "credentials" if value.is_object() || value.is_null() => {}
                    "apiAudience" if value.is_string() => {}
                    "lifetime" if value.as_i64().is_some() => {}
                    "eagerRefreshThresholdMillis" if value.as_u64().is_some() => {}
                    "forceRefreshOnFailure" | "useAuthRequestParameters" if value.is_boolean() => {}
                    _ => {
                        return Err(AiMuxError::UnsupportedFunctionality(format!(
                            "Google auth client option {name} is not supported"
                        )));
                    }
                }
            }
        }
        let api_key = self
            .options
            .api_key
            .as_deref()
            .or_else(|| {
                client
                    .and_then(|options| options.get("apiKey"))
                    .and_then(Value::as_str)
            })
            .filter(|key| !key.is_empty());
        if api_key.is_some() {
            return Err(
                if self.options.credentials.is_some()
                    || client
                        .and_then(|options| options.get("credentials"))
                        .is_some_and(|value| !value.is_null())
                {
                    AiMuxError::InvalidArgument("API Keys and Credentials are mutually exclusive authentication methods and cannot be used together.".into())
                } else {
                    AiMuxError::UnsupportedFunctionality("Google auth API-key clients cannot generate OAuth access tokens; use the Vertex provider api_key setting".into())
                },
            );
        }
        Ok(())
    }

    async fn exchange(
        &self,
        fields: Vec<(&str, String)>,
        url: &str,
    ) -> Result<TokenResponse, AiMuxError> {
        self.exchange_headers(fields, url, Vec::new()).await
    }

    async fn exchange_headers(
        &self,
        fields: Vec<(&str, String)>,
        url: &str,
        headers: Vec<(String, String)>,
    ) -> Result<TokenResponse, AiMuxError> {
        let body = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(fields)
            .finish();
        post_to_api(
            HttpRequest {
                url: url.into(),
                headers,
                fetch: self.fetch.clone(),
                ..Default::default()
            },
            HttpBody::Bytes(
                body.into_bytes(),
                "application/x-www-form-urlencoded".into(),
            ),
            create_json_response_handler::<TokenResponse>(),
            crate::google::google_failed_response_handler(),
        )
        .await
        .map(|response| response.value)
    }
}

pub(super) fn required<'a>(credentials: &'a Value, name: &str) -> Result<&'a str, AiMuxError> {
    credentials
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AiMuxError::InvalidArgument(format!("Google application credentials require {name}"))
        })
}

fn basic_auth(credentials: &Value) -> Result<Vec<(String, String)>, AiMuxError> {
    match credentials.get("client_id").and_then(Value::as_str) {
        Some(id) => Ok(vec![(
            "Authorization".into(),
            format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD
                    .encode(format!("{id}:{}", required(credentials, "client_secret")?))
            ),
        )]),
        None => Ok(Vec::new()),
    }
}

fn der_wrap(tag: u8, bytes: Vec<u8>) -> Vec<u8> {
    let mut result = vec![tag];
    if bytes.len() < 128 {
        result.push(bytes.len() as u8);
    } else {
        let length = bytes.len().to_be_bytes();
        let length = &length[length
            .iter()
            .position(|byte| *byte != 0)
            .unwrap_or(length.len() - 1)..];
        result.push(0x80 | length.len() as u8);
        result.extend_from_slice(length);
    }
    result.extend(bytes);
    result
}

fn normalize_external_credentials(mut credentials: Value) -> Value {
    aliases(
        &mut credentials,
        &[
            "token_url",
            "subject_token_type",
            "client_id",
            "client_secret",
            "workforce_pool_user_project",
            "service_account_impersonation_url",
            "service_account_impersonation",
            "universe_domain",
            "credential_source",
        ],
    );
    if let Some(impersonation) = credentials.get_mut("service_account_impersonation") {
        aliases(impersonation, &["token_lifetime_seconds"]);
    }
    if let Some(source) = credentials.get_mut("credential_source") {
        aliases(
            source,
            &[
                "region_url",
                "regional_cred_verification_url",
                "imdsv2_session_token_url",
            ],
        );
        if let Some(format) = source.get_mut("format") {
            aliases(format, &["subject_token_field_name"]);
        }
    }
    credentials
}

fn aliases(value: &mut Value, fields: &[&str]) {
    let Some(value) = value.as_object_mut() else {
        return;
    };
    for field in fields {
        if value.get(*field).is_some_and(|value| !value.is_null()) {
            continue;
        }
        let mut parts = field.split('_');
        let mut camel = parts.next().unwrap_or_default().to_string();
        for part in parts {
            let mut chars = part.chars();
            if let Some(first) = chars.next() {
                camel.extend(first.to_uppercase());
                camel.extend(chars);
            }
        }
        if let Some(alias) = value.get(&camel).cloned() {
            value.insert((*field).into(), alias);
        }
    }
}
