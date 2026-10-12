//! Port of `ai/src/registry/custom-provider.test.ts`, plus the
//! `custom evaluation models` and `evaluation registry` cases of
//! `ai/src/registry/evaluation-model.test.ts` that apply here.
//!
//! Not ported, with the reason:
//! - `should convert v2 and v3 ... to v4 on demand` and the v2/v3 fallback
//!   cases: there is one model-trait version, so nothing is converted.
//! - `string model ids` and `resolves string aliases through the configured
//!   default provider`: Rust has no global default provider, so an alias
//!   table holds models only.
//! - The Gateway, `evaluate()` and spec-version cases of
//!   `evaluation-model.test.ts`: `evaluate()` and version checks are not part
//!   of this port.
//! - The inherited-property alias names (`toString`, `constructor`,
//!   `__proto__`) are kept in the `reports missing alias` loop although a
//!   Rust map has no inherited keys.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel, EmbeddingResult};
use aimux_core::error::AiMuxError;
use aimux_core::evaluation_model::{
    EvaluationCallOptions, EvaluationModel, EvaluationQuestionType, EvaluationResult,
};
use aimux_core::files_model::{Files, UploadFileCallOptions, UploadFileResult};
use aimux_core::image_model::{ImageCallOptions, ImageModel, ImageResult};
use aimux_core::language_model::LanguageModel;
use aimux_core::options::CallOptions;
use aimux_core::provider::Provider;
use aimux_core::provider_registry::{ProviderRegistryOptions, create_provider_registry};
use aimux_core::reranking_model::{RerankingCallOptions, RerankingModel, RerankingResult};
use aimux_core::result::{GenerateResult, StreamResult};
use aimux_core::search_model::{SearchCallOptions, SearchModel, SearchResult};
use aimux_core::skills_model::{Skills, UploadSkillCallOptions, UploadSkillResult};
use aimux_core::speech_model::{SpeechCallOptions, SpeechModel, SpeechResult};
use aimux_core::transcription_model::{
    TranscriptionCallOptions, TranscriptionModel, TranscriptionResult,
};
use aimux_core::video_model::{
    VideoCallOptions, VideoModel, VideoOperationStart, VideoOperationStatus,
};
use aimux_core::{CustomProviderOptions, custom_provider};

/// A stub whose only identity is the id it was created with; none of its
/// operations is ever called.
macro_rules! stub {
    ($name:ident: $trait:ident { $($item:item)* }) => {
        struct $name(&'static str);
        #[async_trait]
        impl $trait for $name {
            fn provider(&self) -> &str {
                "mock-provider"
            }
            fn model_id(&self) -> &str {
                self.0
            }
            $($item)*
        }
    };
}

stub!(MockLanguage: LanguageModel {
    async fn do_generate(&self, _: &CallOptions) -> Result<GenerateResult, AiMuxError> { unreachable!() }
    async fn do_stream(&self, _: &CallOptions) -> Result<StreamResult, AiMuxError> { unreachable!() }
});
stub!(MockEmbedding: EmbeddingModel {
    fn max_embeddings_per_call(&self) -> Option<u32> { None }
    fn supports_parallel_calls(&self) -> bool { false }
    async fn do_embed(&self, _: &EmbeddingCallOptions) -> Result<EmbeddingResult, AiMuxError> { unreachable!() }
});
stub!(MockImage: ImageModel {
    fn max_images_per_call(&self) -> Option<u32> { None }
    async fn do_generate(&self, _: &ImageCallOptions) -> Result<ImageResult, AiMuxError> { unreachable!() }
});
stub!(MockTranscription: TranscriptionModel {
    async fn do_generate(&self, _: &TranscriptionCallOptions) -> Result<TranscriptionResult, AiMuxError> { unreachable!() }
});
stub!(MockSpeech: SpeechModel {
    async fn do_generate(&self, _: &SpeechCallOptions) -> Result<SpeechResult, AiMuxError> { unreachable!() }
});
stub!(MockReranking: RerankingModel {
    async fn do_rerank(&self, _: &RerankingCallOptions) -> Result<RerankingResult, AiMuxError> { unreachable!() }
});
stub!(MockVideo: VideoModel {
    fn max_videos_per_call(&self) -> Option<u32> { None }
    async fn do_start(&self, _: &VideoCallOptions) -> Result<VideoOperationStart, AiMuxError> { unreachable!() }
    async fn do_status(&self, _: &serde_json::Value, _: &VideoCallOptions) -> Result<VideoOperationStatus, AiMuxError> { unreachable!() }
});
stub!(MockSearch: SearchModel {
    async fn do_search(&self, _: &SearchCallOptions) -> Result<SearchResult, AiMuxError> { unreachable!() }
});
stub!(MockEvaluation: EvaluationModel {
    fn supported_question_types(&self) -> &[EvaluationQuestionType] { &[] }
    async fn do_evaluate(&self, _: &EvaluationCallOptions) -> Result<EvaluationResult, AiMuxError> { unreachable!() }
});

struct MockFiles;
#[async_trait]
impl Files for MockFiles {
    fn provider(&self) -> &str {
        "mock-provider"
    }
    async fn upload_file(&self, _: &UploadFileCallOptions) -> Result<UploadFileResult, AiMuxError> {
        unreachable!()
    }
}

struct MockSkills;
#[async_trait]
impl Skills for MockSkills {
    fn provider(&self) -> &str {
        "mock-provider"
    }
    async fn upload_skill(
        &self,
        _: &UploadSkillCallOptions,
    ) -> Result<UploadSkillResult, AiMuxError> {
        unreachable!()
    }
}

/// A provider that serves a `"fallback"` model for every modality and records
/// each lookup as `"{method}:{id}"`; `evaluation`, `files` and `skills` choose
/// whether it offers those at all.
#[derive(Default)]
struct Fallback {
    calls: Mutex<Vec<String>>,
    evaluation: bool,
    files: bool,
    skills: bool,
}

impl Fallback {
    fn record(&self, method: &str, id: &str) {
        self.calls.lock().unwrap().push(format!("{method}:{id}"));
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl Provider for Fallback {
    fn language_model(&self, id: &str) -> Result<Arc<dyn LanguageModel>, AiMuxError> {
        self.record("language_model", id);
        Ok(Arc::new(MockLanguage("fallback")))
    }
    fn embedding_model(&self, id: &str) -> Result<Arc<dyn EmbeddingModel>, AiMuxError> {
        self.record("embedding_model", id);
        Ok(Arc::new(MockEmbedding("fallback")))
    }
    fn image_model(&self, id: &str) -> Result<Arc<dyn ImageModel>, AiMuxError> {
        self.record("image_model", id);
        Ok(Arc::new(MockImage("fallback")))
    }
    fn transcription_model(
        &self,
        id: &str,
    ) -> Option<Result<Arc<dyn TranscriptionModel>, AiMuxError>> {
        self.record("transcription_model", id);
        Some(Ok(Arc::new(MockTranscription("fallback"))))
    }
    fn speech_model(&self, id: &str) -> Option<Result<Arc<dyn SpeechModel>, AiMuxError>> {
        self.record("speech_model", id);
        Some(Ok(Arc::new(MockSpeech("fallback"))))
    }
    fn reranking_model(&self, id: &str) -> Option<Result<Arc<dyn RerankingModel>, AiMuxError>> {
        self.record("reranking_model", id);
        Some(Ok(Arc::new(MockReranking("fallback"))))
    }
    fn video_model(&self, id: &str) -> Option<Result<Arc<dyn VideoModel>, AiMuxError>> {
        self.record("video_model", id);
        Some(Ok(Arc::new(MockVideo("fallback"))))
    }
    fn search_model(&self, id: &str) -> Option<Result<Arc<dyn SearchModel>, AiMuxError>> {
        self.record("search_model", id);
        Some(Ok(Arc::new(MockSearch("fallback"))))
    }
    fn evaluation_model(&self, id: &str) -> Option<Result<Arc<dyn EvaluationModel>, AiMuxError>> {
        self.record("evaluation_model", id);
        self.evaluation
            .then(|| Ok(Arc::new(MockEvaluation("fallback")) as _))
    }
    fn files(&self) -> Option<Arc<dyn Files>> {
        self.record("files", "");
        self.files.then(|| Arc::new(MockFiles) as _)
    }
    fn skills(&self) -> Option<Arc<dyn Skills>> {
        self.record("skills", "");
        self.skills.then(|| Arc::new(MockSkills) as _)
    }
}

fn assert_no_such_model(result: Result<impl Sized, AiMuxError>, id: &str, model_type: &str) {
    match result {
        Err(AiMuxError::NoSuchModel {
            model_id,
            model_type: t,
        }) => {
            assert_eq!((model_id.as_str(), t.as_str()), (id, model_type));
        }
        Err(other) => panic!("expected NoSuchModel, got {other:?}"),
        Ok(_) => panic!("expected NoSuchModel, got a model"),
    }
}

/// The three cases every modality has: the aliased model, the fallback, and
/// `NoSuchModel` without either. `$get` looks `"test-model"` up on a provider.
macro_rules! modality_cases {
    ($group:ident, $mock:ident, $field:ident, $model_type:literal, $get:expr) => {
        mod $group {
            use super::*;

            /// TS: should return the model if it exists
            #[test]
            fn returns_the_aliased_model() {
                let provider = custom_provider(CustomProviderOptions {
                    $field: HashMap::from([(
                        "test-model".to_string(),
                        Arc::new($mock("table")) as _,
                    )]),
                    ..Default::default()
                });
                assert_eq!(($get)(&provider).unwrap().model_id(), "table");
            }

            /// TS: should use fallback provider if model not found and fallback exists
            #[test]
            fn uses_the_fallback_provider() {
                let fallback = Arc::new(Fallback {
                    evaluation: true,
                    ..Default::default()
                });
                let provider = custom_provider(CustomProviderOptions {
                    fallback_provider: Some(fallback.clone()),
                    ..Default::default()
                });
                assert_eq!(($get)(&provider).unwrap().model_id(), "fallback");
                assert_eq!(fallback.calls().len(), 1);
                assert!(fallback.calls()[0].ends_with(":test-model"));
            }

            /// TS: should throw NoSuchModelError if model not found and no fallback
            #[test]
            fn reports_no_such_model_without_a_fallback() {
                let provider = custom_provider(CustomProviderOptions::default());
                assert_no_such_model(($get)(&provider), "test-model", $model_type);
            }
        }
    };
}

modality_cases!(
    language_model,
    MockLanguage,
    language_models,
    "languageModel",
    |p: &aimux_core::CustomProvider| p.language_model("test-model")
);
modality_cases!(
    embedding_model,
    MockEmbedding,
    embedding_models,
    "embeddingModel",
    |p: &aimux_core::CustomProvider| p.embedding_model("test-model")
);
modality_cases!(
    image_model,
    MockImage,
    image_models,
    "imageModel",
    |p: &aimux_core::CustomProvider| p.image_model("test-model")
);
modality_cases!(
    transcription_model,
    MockTranscription,
    transcription_models,
    "transcriptionModel",
    |p: &aimux_core::CustomProvider| p.transcription_model("test-model").unwrap()
);
modality_cases!(
    speech_model,
    MockSpeech,
    speech_models,
    "speechModel",
    |p: &aimux_core::CustomProvider| p.speech_model("test-model").unwrap()
);
modality_cases!(
    reranking_model,
    MockReranking,
    reranking_models,
    "rerankingModel",
    |p: &aimux_core::CustomProvider| p.reranking_model("test-model").unwrap()
);
modality_cases!(
    video_model,
    MockVideo,
    video_models,
    "videoModel",
    |p: &aimux_core::CustomProvider| p.video_model("test-model").unwrap()
);
modality_cases!(
    search_model,
    MockSearch,
    search_models,
    "searchModel",
    |p: &aimux_core::CustomProvider| p.search_model("test-model").unwrap()
);
modality_cases!(
    evaluation_model,
    MockEvaluation,
    evaluation_models,
    "evaluationModel",
    |p: &aimux_core::CustomProvider| p.evaluation_model("test-model").unwrap()
);

/// TS: should return the files interface if it exists
#[test]
fn returns_the_files_interface() {
    let provider = custom_provider(CustomProviderOptions {
        files: Some(Arc::new(MockFiles)),
        ..Default::default()
    });
    assert!(provider.files().is_some());
}

/// TS: should use fallback provider files if files is not configured and fallback exists
#[test]
fn uses_fallback_provider_files() {
    let fallback = Arc::new(Fallback {
        files: true,
        ..Default::default()
    });
    let provider = custom_provider(CustomProviderOptions {
        fallback_provider: Some(fallback.clone()),
        ..Default::default()
    });
    assert!(provider.files().is_some());
    assert_eq!(fallback.calls(), ["files:"]);
}

/// TS: should not expose files if files is not configured and fallback does not support files
#[test]
fn does_not_expose_files_without_configuration_or_fallback_support() {
    assert!(
        custom_provider(CustomProviderOptions::default())
            .files()
            .is_none()
    );
    let fallback = Arc::new(Fallback::default());
    let provider = custom_provider(CustomProviderOptions {
        fallback_provider: Some(fallback),
        ..Default::default()
    });
    assert!(provider.files().is_none());
}

/// TS: should return the skills interface if it exists
#[test]
fn returns_the_skills_interface() {
    let provider = custom_provider(CustomProviderOptions {
        skills: Some(Arc::new(MockSkills)),
        ..Default::default()
    });
    assert!(provider.skills().is_some());
}

/// TS: should use fallback provider skills if skills is not configured and fallback exists
#[test]
fn uses_fallback_provider_skills() {
    let fallback = Arc::new(Fallback {
        skills: true,
        ..Default::default()
    });
    let provider = custom_provider(CustomProviderOptions {
        fallback_provider: Some(fallback.clone()),
        ..Default::default()
    });
    assert!(provider.skills().is_some());
    assert_eq!(fallback.calls(), ["skills:"]);
}

/// TS: should not expose skills if skills is not configured and fallback does not support skills
#[test]
fn does_not_expose_skills_without_configuration_or_fallback_support() {
    assert!(
        custom_provider(CustomProviderOptions::default())
            .skills()
            .is_none()
    );
    let fallback = Arc::new(Fallback::default());
    let provider = custom_provider(CustomProviderOptions {
        fallback_provider: Some(fallback),
        ..Default::default()
    });
    assert!(provider.skills().is_none());
}

// ── evaluation-model.test.ts ────────────────────────────────────────────────

/// TS: resolves an alias before consulting the fallback
#[test]
fn evaluation_alias_wins_over_the_fallback() {
    let fallback = Arc::new(Fallback {
        evaluation: true,
        ..Default::default()
    });
    let provider = custom_provider(CustomProviderOptions {
        evaluation_models: HashMap::from([(
            "alias".to_string(),
            Arc::new(MockEvaluation("table")) as _,
        )]),
        fallback_provider: Some(fallback.clone()),
        ..Default::default()
    });
    assert_eq!(
        provider
            .evaluation_model("alias")
            .unwrap()
            .unwrap()
            .model_id(),
        "table"
    );
    assert!(fallback.calls().is_empty());
}

/// TS: preserves the fallback receiver
#[test]
fn evaluation_fallback_serves_ids_missing_from_the_table() {
    let fallback = Arc::new(Fallback {
        evaluation: true,
        ..Default::default()
    });
    let provider = custom_provider(CustomProviderOptions {
        fallback_provider: Some(fallback.clone()),
        ..Default::default()
    });
    assert_eq!(
        provider
            .evaluation_model("model:version")
            .unwrap()
            .unwrap()
            .model_id(),
        "fallback"
    );
    assert_eq!(fallback.calls(), ["evaluation_model:model:version"]);
}

/// TS: reports missing alias %s without treating inherited properties as models
#[test]
fn evaluation_reports_missing_aliases() {
    let provider = custom_provider(CustomProviderOptions {
        evaluation_models: HashMap::from([(
            "known".to_string(),
            Arc::new(MockEvaluation("table")) as _,
        )]),
        ..Default::default()
    });
    for id in ["missing", "toString", "constructor", "__proto__"] {
        assert_no_such_model(
            provider.evaluation_model(id).unwrap(),
            id,
            "evaluationModel",
        );
    }
}

/// TS: reports an unavailable fallback model
#[test]
fn evaluation_reports_an_unavailable_fallback_model() {
    let provider = custom_provider(CustomProviderOptions {
        fallback_provider: Some(Arc::new(Fallback::default())),
        ..Default::default()
    });
    assert_no_such_model(
        provider.evaluation_model("missing").unwrap(),
        "missing",
        "evaluationModel",
    );
}

fn registry(
    providers: Vec<(&str, Arc<dyn Provider>)>,
    separator: &str,
) -> aimux_core::ProviderRegistry {
    create_provider_registry(
        providers
            .into_iter()
            .map(|(id, p)| (id.to_string(), p))
            .collect::<BTreeMap<_, _>>(),
        ProviderRegistryOptions {
            separator: separator.to_string(),
            ..Default::default()
        },
    )
}

/// TS: preserves model identity, receiver, and colons inside model IDs
#[test]
fn registry_keeps_colons_inside_model_ids() {
    let provider = Arc::new(Fallback {
        evaluation: true,
        ..Default::default()
    });
    let registry = registry(vec![("provider", provider.clone())], ":");
    let model = registry.evaluation_model("provider:model:version").unwrap();
    assert_eq!(model.model_id(), "fallback");
    assert_eq!(provider.calls(), ["evaluation_model:model:version"]);
}

/// TS: preserves legacy provider extensions and custom separators
#[test]
fn registry_honors_a_custom_separator() {
    let provider = Arc::new(Fallback {
        evaluation: true,
        ..Default::default()
    });
    let registry = registry(vec![("provider", provider.clone())], "::");
    registry
        .evaluation_model("provider::model::version")
        .unwrap();
    assert_eq!(provider.calls(), ["evaluation_model:model::version"]);
}

/// TS: identifies unknown providers with the existing marker-based error
#[test]
fn registry_identifies_unknown_providers() {
    let registry = registry(vec![("known", Arc::new(Fallback::default()))], ":");
    match registry.evaluation_model("missing:model") {
        Err(AiMuxError::NoSuchProvider {
            provider_id,
            model_type,
            available_providers,
            ..
        }) => {
            assert_eq!(provider_id, "missing");
            assert_eq!(model_type, "evaluationModel");
            assert_eq!(available_providers, ["known"]);
        }
        other => panic!("expected NoSuchProvider, got {:?}", other.err()),
    }
}

/// TS: reports malformed IDs
#[test]
fn registry_reports_malformed_ids() {
    assert_no_such_model(
        registry(vec![], ":").evaluation_model("model"),
        "model",
        "evaluationModel",
    );
}

/// TS: reports missing evaluation capabilities or models
#[test]
fn registry_reports_missing_evaluation_models() {
    // A provider that offers no evaluation models at all, and one that
    // declines this id, are the same to the registry.
    let registry = registry(vec![("provider", Arc::new(Fallback::default()))], ":");
    assert_no_such_model(
        registry.evaluation_model("provider:model"),
        "provider:model",
        "evaluationModel",
    );
}
