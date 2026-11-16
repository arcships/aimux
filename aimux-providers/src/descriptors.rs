//! Secret-free static package descriptors for native packages, protocol factories, and presets.

use std::sync::OnceLock;

use aimux_provider_utils::{
    ModalityKind, PackageAuthKind, PackageDescriptor, PackageKind, SettingFieldKind,
    SettingFieldSpec, TemplateParamDescriptor,
};

use crate::{
    anthropic, assemblyai, azure, bedrock, black_forest_labs, cartesia, cohere, dataforseo,
    decision::runtime as runtime_decision,
    deepgram, deepseek, elevenlabs, exa_ai, fal, firecrawl, gladia, google, google_pse, groq,
    huggingface, hume, jina_ai, klingai, linkup, lmnt, luma, mistral, openai, parallel_ai,
    preset::{self, AuthMode},
    prodia, recraft, replicate, revai, runwayml, serper, stability, tavily, tinyfish, vertex,
    voyage, xai, you_com,
};

struct NativeSpec {
    id: &'static str,
    auth_kind: PackageAuthKind,
    api_key_env: Option<&'static str>,
    base_url_env: Option<&'static str>,
    default_base_url: Option<&'static str>,
    default_method: Option<ModalityKind>,
    modalities: &'static [ModalityKind],
    named_language_methods: &'static [&'static str],
    supports_discovery: bool,
    supports_files: bool,
    supports_skills: bool,
    settings: fn(&NativeSpec) -> Vec<SettingFieldSpec>,
}

fn common_settings_with_name(spec: &NativeSpec) -> Vec<SettingFieldSpec> {
    vec![
        SettingFieldSpec {
            name: "apiKey".into(),
            kind: SettingFieldKind::Credential,
            required: false,
            credential: true,
            env_var: spec.api_key_env.map(str::to_owned),
        },
        SettingFieldSpec {
            name: "baseUrl".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: spec.base_url_env.map(str::to_owned),
        },
        SettingFieldSpec {
            name: "headers".into(),
            kind: SettingFieldKind::HeaderMap,
            required: false,
            credential: true,
            env_var: None,
        },
        SettingFieldSpec {
            name: "name".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: None,
        },
    ]
}

fn common_settings_without_name(spec: &NativeSpec) -> Vec<SettingFieldSpec> {
    vec![
        SettingFieldSpec {
            name: "apiKey".into(),
            kind: SettingFieldKind::Credential,
            required: false,
            credential: true,
            env_var: spec.api_key_env.map(str::to_owned),
        },
        SettingFieldSpec {
            name: "baseUrl".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: spec.base_url_env.map(str::to_owned),
        },
        SettingFieldSpec {
            name: "headers".into(),
            kind: SettingFieldKind::HeaderMap,
            required: false,
            credential: true,
            env_var: None,
        },
    ]
}

fn aws_credential_fields() -> Vec<SettingFieldSpec> {
    vec![
        SettingFieldSpec {
            name: "region".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: Some("AWS_REGION".into()),
        },
        SettingFieldSpec {
            name: "accessKeyId".into(),
            kind: SettingFieldKind::Credential,
            required: false,
            credential: true,
            env_var: Some("AWS_ACCESS_KEY_ID".into()),
        },
        SettingFieldSpec {
            name: "secretAccessKey".into(),
            kind: SettingFieldKind::Credential,
            required: false,
            credential: true,
            env_var: Some("AWS_SECRET_ACCESS_KEY".into()),
        },
        SettingFieldSpec {
            name: "sessionToken".into(),
            kind: SettingFieldKind::Credential,
            required: false,
            credential: true,
            env_var: Some("AWS_SESSION_TOKEN".into()),
        },
    ]
}

fn openai_settings(spec: &NativeSpec) -> Vec<SettingFieldSpec> {
    let mut fields = common_settings_with_name(spec);
    fields.extend([
        SettingFieldSpec {
            name: "organization".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: None,
        },
        SettingFieldSpec {
            name: "project".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: None,
        },
    ]);
    fields
}

fn anthropic_settings(spec: &NativeSpec) -> Vec<SettingFieldSpec> {
    let mut fields = common_settings_with_name(spec);
    fields.insert(
        1,
        SettingFieldSpec {
            name: "authToken".into(),
            kind: SettingFieldKind::Credential,
            required: false,
            credential: true,
            env_var: None,
        },
    );
    fields
}

fn azure_settings(spec: &NativeSpec) -> Vec<SettingFieldSpec> {
    let mut fields = common_settings_without_name(spec);
    fields.extend([
        SettingFieldSpec {
            name: "accessToken".into(),
            kind: SettingFieldKind::Credential,
            required: false,
            credential: true,
            env_var: None,
        },
        SettingFieldSpec {
            name: "resourceName".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: Some("AZURE_RESOURCE_NAME".into()),
        },
        SettingFieldSpec {
            name: "speechBaseUrl".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: None,
        },
        SettingFieldSpec {
            name: "apiVersion".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: None,
        },
        SettingFieldSpec {
            name: "useDeploymentBasedUrls".into(),
            kind: SettingFieldKind::Boolean,
            required: false,
            credential: false,
            env_var: None,
        },
    ]);
    fields
}

fn bedrock_settings(spec: &NativeSpec) -> Vec<SettingFieldSpec> {
    let mut fields = common_settings_without_name(spec);
    fields.extend(aws_credential_fields());
    fields
}

fn anthropic_aws_settings(_spec: &NativeSpec) -> Vec<SettingFieldSpec> {
    let mut fields = vec![
        SettingFieldSpec {
            name: "apiKey".into(),
            kind: SettingFieldKind::Credential,
            required: false,
            credential: true,
            env_var: Some("ANTHROPIC_AWS_API_KEY".into()),
        },
        SettingFieldSpec {
            name: "workspaceId".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: Some("ANTHROPIC_AWS_WORKSPACE_ID".into()),
        },
        SettingFieldSpec {
            name: "baseUrl".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: None,
        },
        SettingFieldSpec {
            name: "headers".into(),
            kind: SettingFieldKind::HeaderMap,
            required: false,
            credential: true,
            env_var: None,
        },
        SettingFieldSpec {
            name: "name".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: None,
        },
    ];
    fields.extend(aws_credential_fields());
    fields
}

fn aws_polly_settings(_spec: &NativeSpec) -> Vec<SettingFieldSpec> {
    let mut fields = aws_credential_fields();
    fields.extend([
        SettingFieldSpec {
            name: "baseUrl".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: None,
        },
        SettingFieldSpec {
            name: "headers".into(),
            kind: SettingFieldKind::HeaderMap,
            required: false,
            credential: true,
            env_var: None,
        },
        SettingFieldSpec {
            name: "name".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: None,
        },
    ]);
    fields
}

fn codex_settings(_spec: &NativeSpec) -> Vec<SettingFieldSpec> {
    vec![
        SettingFieldSpec {
            name: "codexMode".into(),
            kind: SettingFieldKind::CodexMode,
            required: false,
            credential: true,
            env_var: Some("CODEX_API_KEY".into()),
        },
        SettingFieldSpec {
            name: "apiKey".into(),
            kind: SettingFieldKind::Credential,
            required: false,
            credential: true,
            env_var: Some("CODEX_API_KEY".into()),
        },
        SettingFieldSpec {
            name: "originator".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: None,
        },
        SettingFieldSpec {
            name: "baseUrl".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: None,
        },
        SettingFieldSpec {
            name: "headers".into(),
            kind: SettingFieldKind::HeaderMap,
            required: false,
            credential: true,
            env_var: None,
        },
        SettingFieldSpec {
            name: "name".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: None,
        },
    ]
}

fn dataforseo_settings(_spec: &NativeSpec) -> Vec<SettingFieldSpec> {
    vec![
        SettingFieldSpec {
            name: "login".into(),
            kind: SettingFieldKind::Credential,
            required: false,
            credential: true,
            env_var: Some("DATAFORSEO_LOGIN".into()),
        },
        SettingFieldSpec {
            name: "password".into(),
            kind: SettingFieldKind::Credential,
            required: false,
            credential: true,
            env_var: Some("DATAFORSEO_PASSWORD".into()),
        },
        SettingFieldSpec {
            name: "baseUrl".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: None,
        },
        SettingFieldSpec {
            name: "headers".into(),
            kind: SettingFieldKind::HeaderMap,
            required: false,
            credential: true,
            env_var: None,
        },
        SettingFieldSpec {
            name: "name".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: None,
        },
    ]
}

fn google_pse_settings(spec: &NativeSpec) -> Vec<SettingFieldSpec> {
    let mut fields = common_settings_with_name(spec);
    fields.push(SettingFieldSpec {
        name: "cx".into(),
        kind: SettingFieldKind::String,
        required: false,
        credential: false,
        env_var: Some("GOOGLE_CSE_ID".into()),
    });
    fields
}

fn vertex_settings(spec: &NativeSpec) -> Vec<SettingFieldSpec> {
    let mut fields = common_settings_without_name(spec);
    fields.extend([
        SettingFieldSpec {
            name: "accessToken".into(),
            kind: SettingFieldKind::Credential,
            required: false,
            credential: true,
            env_var: Some("GOOGLE_VERTEX_ACCESS_TOKEN".into()),
        },
        SettingFieldSpec {
            name: "location".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: Some("GOOGLE_VERTEX_LOCATION".into()),
        },
        SettingFieldSpec {
            name: "project".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: Some("GOOGLE_VERTEX_PROJECT".into()),
        },
    ]);
    fields
}

const BEDROCK_MODALITIES: &[ModalityKind] = &[
    ModalityKind::LanguageModel,
    ModalityKind::EmbeddingModel,
    ModalityKind::ImageModel,
    ModalityKind::RerankingModel,
];
const ANTHROPIC_MODALITIES: &[ModalityKind] =
    &[ModalityKind::LanguageModel, ModalityKind::Decision];
const LANG_ONLY: &[ModalityKind] = &[ModalityKind::LanguageModel];
const AZURE_MODALITIES: &[ModalityKind] = &[
    ModalityKind::LanguageModel,
    ModalityKind::EmbeddingModel,
    ModalityKind::ImageModel,
    ModalityKind::SpeechModel,
    ModalityKind::TranscriptionModel,
];
const CARTESIA_ELEVEN_MODALITIES: &[ModalityKind] =
    &[ModalityKind::SpeechModel, ModalityKind::TranscriptionModel];
const COHERE_MODALITIES: &[ModalityKind] = &[
    ModalityKind::LanguageModel,
    ModalityKind::EmbeddingModel,
    ModalityKind::RerankingModel,
];
const FAL_MODALITIES: &[ModalityKind] = &[
    ModalityKind::ImageModel,
    ModalityKind::TranscriptionModel,
    ModalityKind::VideoModel,
];
const GOOGLE_MODALITIES: &[ModalityKind] = &[
    ModalityKind::LanguageModel,
    ModalityKind::EmbeddingModel,
    ModalityKind::ImageModel,
    ModalityKind::SpeechModel,
    ModalityKind::TranscriptionModel,
    ModalityKind::VideoModel,
    ModalityKind::Decision,
];
const VERTEX_MODALITIES: &[ModalityKind] = &[
    ModalityKind::LanguageModel,
    ModalityKind::EmbeddingModel,
    ModalityKind::ImageModel,
    ModalityKind::SpeechModel,
    ModalityKind::TranscriptionModel,
    ModalityKind::VideoModel,
];
const GROQ_MODALITIES: &[ModalityKind] = &[
    ModalityKind::LanguageModel,
    ModalityKind::TranscriptionModel,
];
const MISTRAL_MODALITIES: &[ModalityKind] = &[
    ModalityKind::LanguageModel,
    ModalityKind::EmbeddingModel,
    ModalityKind::SpeechModel,
    ModalityKind::TranscriptionModel,
];
const OPENAI_MODALITIES: &[ModalityKind] = &[
    ModalityKind::LanguageModel,
    ModalityKind::EmbeddingModel,
    ModalityKind::ImageModel,
    ModalityKind::SpeechModel,
    ModalityKind::TranscriptionModel,
    ModalityKind::Decision,
];
const PRODIA_REPLICATE_MODALITIES: &[ModalityKind] =
    &[ModalityKind::ImageModel, ModalityKind::VideoModel];
const VOYAGE_MODALITIES: &[ModalityKind] =
    &[ModalityKind::EmbeddingModel, ModalityKind::RerankingModel];
const XAI_MODALITIES: &[ModalityKind] = &[
    ModalityKind::LanguageModel,
    ModalityKind::SpeechModel,
    ModalityKind::TranscriptionModel,
    ModalityKind::VideoModel,
];
const IMAGE_ONLY: &[ModalityKind] = &[ModalityKind::ImageModel];
const SPEECH_ONLY: &[ModalityKind] = &[ModalityKind::SpeechModel];
const TRANSCRIPTION_ONLY: &[ModalityKind] = &[ModalityKind::TranscriptionModel];
const RERANKING_ONLY: &[ModalityKind] = &[ModalityKind::RerankingModel];
const VIDEO_ONLY: &[ModalityKind] = &[ModalityKind::VideoModel];
const SEARCH_ONLY: &[ModalityKind] = &[ModalityKind::SearchModel];

const NATIVE_SPECS: &[NativeSpec] = &[
    NativeSpec {
        id: "amazon_bedrock",
        auth_kind: PackageAuthKind::AwsBearerOrSigV4,
        api_key_env: Some(bedrock::BEARER_TOKEN_ENV_VAR),
        base_url_env: Some("AWS_ENDPOINT_URL_BEDROCK_RUNTIME"),
        default_base_url: None,
        default_method: Some(ModalityKind::LanguageModel),
        modalities: BEDROCK_MODALITIES,
        named_language_methods: &["chat"],
        supports_discovery: true,
        supports_files: false,
        supports_skills: false,
        settings: bedrock_settings,
    },
    NativeSpec {
        id: "anthropic",
        auth_kind: PackageAuthKind::Header,
        api_key_env: Some(anthropic::API_KEY_ENV_VAR),
        base_url_env: Some("ANTHROPIC_BASE_URL"),
        default_base_url: Some(anthropic::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::LanguageModel),
        modalities: ANTHROPIC_MODALITIES,
        named_language_methods: &["messages", "chat"],
        supports_discovery: true,
        supports_files: true,
        supports_skills: true,
        settings: anthropic_settings,
    },
    NativeSpec {
        id: "anthropic_aws",
        auth_kind: PackageAuthKind::AnthropicAws,
        api_key_env: None,
        base_url_env: None,
        default_base_url: None,
        default_method: Some(ModalityKind::LanguageModel),
        modalities: LANG_ONLY,
        named_language_methods: &["messages", "chat"],
        supports_discovery: true,
        supports_files: false,
        supports_skills: false,
        settings: anthropic_aws_settings,
    },
    NativeSpec {
        id: "assemblyai",
        auth_kind: PackageAuthKind::Header,
        api_key_env: Some(assemblyai::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(assemblyai::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::TranscriptionModel),
        modalities: TRANSCRIPTION_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "aws_polly",
        auth_kind: PackageAuthKind::AwsSigV4,
        api_key_env: None,
        base_url_env: None,
        default_base_url: None,
        default_method: Some(ModalityKind::SpeechModel),
        modalities: SPEECH_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: aws_polly_settings,
    },
    NativeSpec {
        id: "azure",
        auth_kind: PackageAuthKind::AzureApiKeyOrToken,
        api_key_env: Some(azure::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: None,
        default_method: Some(ModalityKind::LanguageModel),
        modalities: AZURE_MODALITIES,
        named_language_methods: &["chat", "responses"],
        supports_discovery: true,
        supports_files: false,
        supports_skills: false,
        settings: azure_settings,
    },
    NativeSpec {
        id: "black_forest_labs",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(black_forest_labs::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(black_forest_labs::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::ImageModel),
        modalities: IMAGE_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "cartesia",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(cartesia::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(cartesia::DEFAULT_BASE_URL),
        default_method: None,
        modalities: CARTESIA_ELEVEN_MODALITIES,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "codex",
        auth_kind: PackageAuthKind::CodexChannel,
        api_key_env: None,
        base_url_env: None,
        default_base_url: None,
        default_method: Some(ModalityKind::LanguageModel),
        modalities: LANG_ONLY,
        named_language_methods: &["responses"],
        supports_discovery: true,
        supports_files: false,
        supports_skills: false,
        settings: codex_settings,
    },
    NativeSpec {
        id: "cohere",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(cohere::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(cohere::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::LanguageModel),
        modalities: COHERE_MODALITIES,
        named_language_methods: &["chat"],
        supports_discovery: true,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_without_name,
    },
    NativeSpec {
        id: "dataforseo",
        auth_kind: PackageAuthKind::Basic,
        api_key_env: None,
        base_url_env: None,
        default_base_url: Some(dataforseo::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::SearchModel),
        modalities: SEARCH_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: dataforseo_settings,
    },
    NativeSpec {
        id: "deepgram",
        auth_kind: PackageAuthKind::Header,
        api_key_env: Some(deepgram::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(deepgram::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::TranscriptionModel),
        modalities: TRANSCRIPTION_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "deepseek",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(deepseek::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(deepseek::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::LanguageModel),
        modalities: LANG_ONLY,
        named_language_methods: &["chat"],
        supports_discovery: true,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_without_name,
    },
    NativeSpec {
        id: "elevenlabs",
        auth_kind: PackageAuthKind::Header,
        api_key_env: Some(elevenlabs::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(elevenlabs::DEFAULT_BASE_URL),
        default_method: None,
        modalities: CARTESIA_ELEVEN_MODALITIES,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "exa_ai",
        auth_kind: PackageAuthKind::Header,
        api_key_env: Some(exa_ai::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(exa_ai::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::SearchModel),
        modalities: SEARCH_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "fal",
        auth_kind: PackageAuthKind::Header,
        api_key_env: Some(fal::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(fal::DEFAULT_BASE_URL),
        default_method: None,
        modalities: FAL_MODALITIES,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "firecrawl",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(firecrawl::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(firecrawl::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::SearchModel),
        modalities: SEARCH_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "gladia",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(gladia::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(gladia::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::TranscriptionModel),
        modalities: TRANSCRIPTION_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "google",
        auth_kind: PackageAuthKind::Header,
        api_key_env: Some(google::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(google::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::LanguageModel),
        modalities: GOOGLE_MODALITIES,
        named_language_methods: &["chat", "generativeAI", "generative_ai"],
        supports_discovery: true,
        supports_files: true,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "google_pse",
        auth_kind: PackageAuthKind::QueryParam,
        api_key_env: Some(google_pse::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(google_pse::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::SearchModel),
        modalities: SEARCH_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: google_pse_settings,
    },
    NativeSpec {
        id: "google_vertex",
        auth_kind: PackageAuthKind::GoogleVertex,
        api_key_env: Some(vertex::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: None,
        default_method: Some(ModalityKind::LanguageModel),
        modalities: VERTEX_MODALITIES,
        named_language_methods: &["chat"],
        supports_discovery: true,
        supports_files: false,
        supports_skills: false,
        settings: vertex_settings,
    },
    NativeSpec {
        id: "groq",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(groq::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(groq::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::LanguageModel),
        modalities: GROQ_MODALITIES,
        named_language_methods: &["chat"],
        supports_discovery: true,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_without_name,
    },
    NativeSpec {
        id: "huggingface",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(huggingface::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(huggingface::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::LanguageModel),
        modalities: LANG_ONLY,
        named_language_methods: &["responses"],
        supports_discovery: true,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "hume",
        auth_kind: PackageAuthKind::Header,
        api_key_env: Some(hume::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(hume::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::SpeechModel),
        modalities: SPEECH_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "jina_ai",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(jina_ai::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(jina_ai::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::RerankingModel),
        modalities: RERANKING_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "klingai",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(klingai::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(klingai::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::VideoModel),
        modalities: VIDEO_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "linkup",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(linkup::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(linkup::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::SearchModel),
        modalities: SEARCH_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "lmnt",
        auth_kind: PackageAuthKind::Header,
        api_key_env: Some(lmnt::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(lmnt::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::SpeechModel),
        modalities: SPEECH_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "luma",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(luma::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(luma::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::ImageModel),
        modalities: IMAGE_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "mistral",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(mistral::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(mistral::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::LanguageModel),
        modalities: MISTRAL_MODALITIES,
        named_language_methods: &["chat"],
        supports_discovery: true,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_without_name,
    },
    NativeSpec {
        id: "openai",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(openai::API_KEY_ENV_VAR),
        base_url_env: Some("OPENAI_BASE_URL"),
        default_base_url: Some(openai::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::LanguageModel),
        modalities: OPENAI_MODALITIES,
        named_language_methods: &["chat", "responses", "completion"],
        supports_discovery: true,
        supports_files: true,
        supports_skills: false,
        settings: openai_settings,
    },
    NativeSpec {
        id: "parallel_ai",
        auth_kind: PackageAuthKind::Header,
        api_key_env: Some(parallel_ai::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(parallel_ai::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::SearchModel),
        modalities: SEARCH_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "prodia",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(prodia::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(prodia::DEFAULT_BASE_URL),
        default_method: None,
        modalities: PRODIA_REPLICATE_MODALITIES,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "recraft",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(recraft::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(recraft::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::ImageModel),
        modalities: IMAGE_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "replicate",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(replicate::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(replicate::DEFAULT_BASE_URL),
        default_method: None,
        modalities: PRODIA_REPLICATE_MODALITIES,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "revai",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(revai::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(revai::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::TranscriptionModel),
        modalities: TRANSCRIPTION_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "runwayml",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(runwayml::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(runwayml::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::VideoModel),
        modalities: VIDEO_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "searxng",
        auth_kind: PackageAuthKind::OptionalBearer,
        api_key_env: None,
        base_url_env: Some("SEARXNG_URL"),
        default_base_url: None,
        default_method: Some(ModalityKind::SearchModel),
        modalities: SEARCH_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "serper",
        auth_kind: PackageAuthKind::Header,
        api_key_env: Some(serper::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(serper::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::SearchModel),
        modalities: SEARCH_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "stability",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(stability::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(stability::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::ImageModel),
        modalities: IMAGE_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "tavily",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(tavily::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(tavily::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::SearchModel),
        modalities: SEARCH_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "tinyfish",
        auth_kind: PackageAuthKind::Header,
        api_key_env: Some(tinyfish::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(tinyfish::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::SearchModel),
        modalities: SEARCH_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "voyage",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(voyage::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(voyage::DEFAULT_BASE_URL),
        default_method: None,
        modalities: VOYAGE_MODALITIES,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
    NativeSpec {
        id: "xai",
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some(xai::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(xai::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::LanguageModel),
        modalities: XAI_MODALITIES,
        named_language_methods: &["responses"],
        supports_discovery: true,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_without_name,
    },
    NativeSpec {
        id: "you_com",
        auth_kind: PackageAuthKind::Header,
        api_key_env: Some(you_com::API_KEY_ENV_VAR),
        base_url_env: None,
        default_base_url: Some(you_com::DEFAULT_BASE_URL),
        default_method: Some(ModalityKind::SearchModel),
        modalities: SEARCH_ONLY,
        named_language_methods: &[],
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: common_settings_with_name,
    },
];

fn native_spec(name: &str) -> Option<&'static NativeSpec> {
    NATIVE_SPECS.iter().find(|spec| spec.id == name)
}

pub(crate) fn native_api_key_env(name: &str) -> Option<&'static str> {
    native_spec(name).and_then(|spec| spec.api_key_env)
}

pub(crate) fn native_default_base_url(name: &str) -> Option<&'static str> {
    native_spec(name).and_then(|spec| spec.default_base_url)
}

fn build_native_descriptor(spec: &NativeSpec) -> PackageDescriptor {
    PackageDescriptor {
        id: spec.id.to_owned(),
        kind: PackageKind::Native,
        auth_kind: spec.auth_kind,
        api_key_env: spec.api_key_env.map(str::to_owned),
        base_url_env: spec.base_url_env.map(str::to_owned),
        default_base_url: spec.default_base_url.map(str::to_owned),
        supports_default_instance: true,
        default_method: spec.default_method,
        modalities: spec.modalities.to_vec(),
        named_language_methods: spec
            .named_language_methods
            .iter()
            .map(|&s| s.to_owned())
            .collect(),
        supports_discovery: spec.supports_discovery,
        supports_files: spec.supports_files,
        supports_skills: spec.supports_skills,
        settings: (spec.settings)(spec),
        template_params: Vec::new(),
    }
}

fn build_openai_compatible_descriptor() -> PackageDescriptor {
    PackageDescriptor {
        id: "openai_compatible".into(),
        kind: PackageKind::Protocol,
        auth_kind: PackageAuthKind::OptionalBearer,
        api_key_env: None,
        base_url_env: None,
        default_base_url: None,
        supports_default_instance: false,
        default_method: Some(ModalityKind::LanguageModel),
        modalities: vec![
            ModalityKind::LanguageModel,
            ModalityKind::EmbeddingModel,
            ModalityKind::ImageModel,
        ],
        named_language_methods: vec!["chat".into(), "completion".into()],
        supports_discovery: true,
        supports_files: false,
        supports_skills: false,
        settings: vec![
            SettingFieldSpec {
                name: "name".into(),
                kind: SettingFieldKind::String,
                required: false,
                credential: false,
                env_var: None,
            },
            SettingFieldSpec {
                name: "baseUrl".into(),
                kind: SettingFieldKind::String,
                required: true,
                credential: false,
                env_var: None,
            },
            SettingFieldSpec {
                name: "apiKey".into(),
                kind: SettingFieldKind::Credential,
                required: false,
                credential: true,
                env_var: None,
            },
            SettingFieldSpec {
                name: "headers".into(),
                kind: SettingFieldKind::HeaderMap,
                required: false,
                credential: true,
                env_var: None,
            },
            SettingFieldSpec {
                name: "queryParams".into(),
                kind: SettingFieldKind::StringMap,
                required: false,
                credential: false,
                env_var: None,
            },
            SettingFieldSpec {
                name: "includeUsage".into(),
                kind: SettingFieldKind::Boolean,
                required: false,
                credential: false,
                env_var: None,
            },
            SettingFieldSpec {
                name: "supportsStructuredOutputs".into(),
                kind: SettingFieldKind::Boolean,
                required: false,
                credential: false,
                env_var: None,
            },
            SettingFieldSpec {
                name: "supportsMultiPartToolContent".into(),
                kind: SettingFieldKind::Boolean,
                required: false,
                credential: false,
                env_var: None,
            },
        ],
        template_params: Vec::new(),
    }
}

fn build_jev_descriptor() -> PackageDescriptor {
    PackageDescriptor {
        id: "jev".into(),
        kind: PackageKind::Specialized,
        auth_kind: PackageAuthKind::Bearer,
        api_key_env: Some("TYPESAFE_API_KEY".into()),
        base_url_env: None,
        default_base_url: Some("https://api.typesafe.ai/v1/systemone".into()),
        supports_default_instance: false,
        default_method: Some(ModalityKind::Decision),
        modalities: vec![ModalityKind::Decision],
        named_language_methods: Vec::new(),
        supports_discovery: false,
        supports_files: false,
        supports_skills: false,
        settings: vec![
            SettingFieldSpec {
                name: "apiKey".into(),
                kind: SettingFieldKind::Credential,
                required: false,
                credential: true,
                env_var: Some("TYPESAFE_API_KEY".into()),
            },
            SettingFieldSpec {
                name: "endpoint".into(),
                kind: SettingFieldKind::String,
                required: false,
                credential: false,
                env_var: None,
            },
            SettingFieldSpec {
                name: "headers".into(),
                kind: SettingFieldKind::HeaderMap,
                required: false,
                credential: true,
                env_var: None,
            },
            SettingFieldSpec {
                name: "probabilitySource".into(),
                kind: SettingFieldKind::String,
                required: false,
                credential: false,
                env_var: None,
            },
        ],
        template_params: Vec::new(),
    }
}

fn build_preset_descriptor(entry: preset::PresetEntry) -> PackageDescriptor {
    let raw = entry.descriptor;
    let api_key_env = (!raw.env_var.is_empty()).then(|| raw.env_var.clone());
    let auth_kind = match raw.auth {
        AuthMode::ApiKey => PackageAuthKind::Bearer,
        AuthMode::None => PackageAuthKind::None,
    };
    let mut modalities = vec![
        ModalityKind::LanguageModel,
        ModalityKind::EmbeddingModel,
        ModalityKind::ImageModel,
    ];
    if runtime_decision::supported(&raw.name) {
        modalities.push(ModalityKind::Decision);
    }
    let template_params: Vec<TemplateParamDescriptor> = raw
        .params
        .iter()
        .filter(|param| param.derive.is_none())
        .map(|param| TemplateParamDescriptor {
            name: param.name.clone(),
            env_var: param.env.first().cloned().unwrap_or_default(),
        })
        .collect();
    let mut settings = vec![
        SettingFieldSpec {
            name: "apiKey".into(),
            kind: SettingFieldKind::Credential,
            required: false,
            credential: true,
            env_var: api_key_env.clone(),
        },
        SettingFieldSpec {
            name: "baseUrl".into(),
            kind: SettingFieldKind::String,
            required: false,
            credential: false,
            env_var: raw.base_url_env.clone(),
        },
        SettingFieldSpec {
            name: "headers".into(),
            kind: SettingFieldKind::HeaderMap,
            required: false,
            credential: true,
            env_var: None,
        },
    ];
    if !template_params.is_empty() {
        settings.push(SettingFieldSpec {
            name: "params".into(),
            kind: SettingFieldKind::StringMap,
            required: false,
            credential: false,
            env_var: None,
        });
    }
    PackageDescriptor {
        id: raw.name.clone(),
        kind: PackageKind::Preset,
        auth_kind,
        api_key_env,
        base_url_env: raw.base_url_env.clone(),
        default_base_url: raw.params.is_empty().then(|| raw.base_url.clone()),
        supports_default_instance: true,
        default_method: Some(ModalityKind::LanguageModel),
        modalities,
        named_language_methods: vec!["chat".into(), "completion".into()],
        supports_discovery: true,
        supports_files: false,
        supports_skills: false,
        settings,
        template_params,
    }
}

fn descriptors_table() -> &'static [PackageDescriptor] {
    static TABLE: OnceLock<Vec<PackageDescriptor>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut list: Vec<PackageDescriptor> =
            NATIVE_SPECS.iter().map(build_native_descriptor).collect();
        list.push(build_openai_compatible_descriptor());
        list.push(build_jev_descriptor());
        for entry in preset::entries() {
            let name = entry.descriptor.name.as_str();
            if native_spec(name).is_some() || name == "openai_compatible" || name == "jev" {
                continue;
            }
            list.push(build_preset_descriptor(*entry));
        }
        list.sort_by(|a, b| a.id.cmp(&b.id));
        list
    })
}

/// Static descriptors for every supported package id (native vendors,
/// `"openai_compatible"`, `"jev"`, and registry presets), sorted by `id`.
#[must_use]
pub fn package_descriptors() -> Vec<PackageDescriptor> {
    descriptors_table().to_vec()
}

/// Look up the static descriptor for `id`.
#[must_use]
pub fn package_descriptor(id: &str) -> Option<PackageDescriptor> {
    let table = descriptors_table();
    table
        .binary_search_by(|candidate| candidate.id.as_str().cmp(id))
        .ok()
        .map(|idx| table[idx].clone())
}
