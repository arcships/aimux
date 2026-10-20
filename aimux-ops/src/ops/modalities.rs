//! Model operations reuse Core's wire types and retry/timeout orchestration.

use aimux_core::{
    embedding_model, image_model, reranking_model, search_model, speech_model, transcription_model,
    video_model,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::dispatch::{CallFuture, Ctx, Handler, Target, decode};
use crate::envelope::{BindingError, OpsError, ProtocolError};
use crate::handle::{Entry, HandleKind};
use crate::op::{Cancellation, OpDescriptor, OpMode, RequestBytePolicy, Scope, TypeRef};

const fn operation(
    name: &'static str,
    kinds: &'static [HandleKind],
    request_type: TypeRef,
    result_type: TypeRef,
    request_byte_policy: RequestBytePolicy,
    source_symbols: &'static [&'static str],
) -> OpDescriptor {
    OpDescriptor {
        name,
        since_ops_version: "1.0",
        mode: OpMode::Call,
        allowed_handle_kinds: kinds,
        request_type,
        result_type,
        event_type: None,
        scope: Scope::Current,
        terminal_type: None,
        runtime_handle_fields: &[("abort", &[HandleKind::Abort])],
        required_capabilities: &[],
        cancellation: Cancellation::Abort,
        result_ownership: "owned",
        request_byte_policy,
        side_effects: true,
        retry_owner: "core",
        source_symbols,
    }
}

pub(crate) static OPS: &[OpDescriptor] = &[
    operation(
        "embedding.embed",
        &[HandleKind::Embedding],
        TypeRef::of::<aimux_core::embedding_model::EmbeddingCallOptions>(),
        TypeRef::of::<aimux_core::embedding_model::EmbeddingResult>(),
        RequestBytePolicy::Json,
        &["aimux_embed", "aimux_core::embedding_model::embed"],
    ),
    operation(
        "embedding.embedMany",
        &[HandleKind::Embedding],
        TypeRef::of::<aimux_core::embedding_model::EmbeddingCallOptions>(),
        TypeRef::of::<aimux_core::embedding_model::EmbeddingResult>(),
        RequestBytePolicy::Json,
        &["aimux_embed", "aimux_core::embedding_model::embed"],
    ),
    operation(
        "image.generate",
        &[HandleKind::Image],
        TypeRef::of::<aimux_core::image_model::ImageCallOptions>(),
        TypeRef::of::<aimux_core::image_model::ImageResult>(),
        RequestBytePolicy::JsonWithBase64,
        &[
            "aimux_image_generate",
            "aimux_core::image_model::generate_image",
        ],
    ),
    operation(
        "speech.generate",
        &[HandleKind::Speech],
        TypeRef::of::<aimux_core::speech_model::SpeechCallOptions>(),
        TypeRef::of::<aimux_core::speech_model::SpeechResult>(),
        RequestBytePolicy::Json,
        &[
            "aimux_speech_generate",
            "aimux_core::speech_model::generate_speech",
        ],
    ),
    operation(
        "transcription.generate",
        &[HandleKind::Transcription],
        TypeRef::of::<aimux_core::transcription_model::TranscriptionCallOptions>(),
        TypeRef::of::<aimux_core::transcription_model::TranscriptionResult>(),
        RequestBytePolicy::JsonWithBase64,
        &[
            "aimux_transcription_generate",
            "aimux_core::transcription_model::transcribe",
        ],
    ),
    operation(
        "reranking.rerank",
        &[HandleKind::Reranking],
        TypeRef::of::<aimux_core::reranking_model::RerankingCallOptions>(),
        TypeRef::of::<aimux_core::reranking_model::RerankingResult>(),
        RequestBytePolicy::Json,
        &["aimux_rerank", "aimux_core::reranking_model::rerank"],
    ),
    operation(
        "video.generate",
        &[HandleKind::Video],
        TypeRef::of::<aimux_core::video_model::VideoCallOptions>(),
        TypeRef::of::<aimux_core::video_model::VideoResult>(),
        RequestBytePolicy::JsonWithBase64,
        &[
            "aimux_video_generate",
            "aimux_core::video_model::generate_video",
        ],
    ),
    operation(
        "search.search",
        &[HandleKind::Search],
        TypeRef::of::<aimux_core::search_model::SearchCallOptions>(),
        TypeRef::of::<aimux_core::search_model::SearchResult>(),
        RequestBytePolicy::Json,
        &["aimux_search", "aimux_core::search_model::search"],
    ),
];

pub(crate) fn register(r: &mut Vec<(&'static OpDescriptor, Handler)>) {
    r.extend([
        (&OPS[0], Handler::Call(embed)),
        (&OPS[1], Handler::Call(embed)),
        (&OPS[2], Handler::Call(generate_image)),
        (&OPS[3], Handler::Call(generate_speech)),
        (&OPS[4], Handler::Call(transcribe)),
        (&OPS[5], Handler::Call(rerank)),
        (&OPS[6], Handler::Call(generate_video)),
        (&OPS[7], Handler::Call(search)),
    ]);
}

// Core options are canonical DTOs with open providerOptions/JSON fields. Keep
// their serde implementations, but reject unknown operation-level control keys.
fn check_fields(params: &Value, fields: &[&str]) -> Result<(), OpsError> {
    let object = params.as_object().ok_or_else(|| {
        OpsError::Protocol(ProtocolError::InvalidParams(
            "expected an options object".into(),
        ))
    })?;
    for key in object.keys() {
        if !fields.contains(&key.as_str()) {
            return Err(OpsError::Protocol(ProtocolError::InvalidParams(format!(
                "unknown field `{key}`"
            ))));
        }
    }
    Ok(())
}

fn options<T: DeserializeOwned>(params: Value, fields: &[&str]) -> Result<T, OpsError> {
    check_fields(&params, fields)?;
    if let Some(timeout) = params.get("timeout").filter(|value| !value.is_null()) {
        check_fields(timeout, &["totalMs", "stepMs", "firstChunkMs", "chunkMs"])?;
    }
    if let Some(poll) = params.get("poll").filter(|value| !value.is_null()) {
        check_fields(poll, &["intervalMs", "timeoutMs"])?;
    }
    decode(params)
}

fn result(value: impl Serialize) -> Result<Value, OpsError> {
    serde_json::to_value(value)
        .map_err(|error| OpsError::Binding(BindingError::ResultSerialization(error.to_string())))
}

macro_rules! model_call {
    ($name:ident, $kind:ident, $module:ident, $options:ident, $call:ident, [$($field:literal),* $(,)?]) => {
        fn $name(ctx: Ctx, target: Target, mut params: Value) -> CallFuture {
            Box::pin(async move {
                let model = match target {
                    Target::Handle(Entry::$kind(model)) => model,
                    other => return Err(OpsError::Binding(BindingError::InvalidHandle {
                        expected: vec![HandleKind::$kind],
                        actual: match other {
                            Target::Handle(entry) => Some(entry.kind()),
                            Target::Global => None,
                        },
                    })),
                };
                // Dispatch has resolved this control through the owner-aware
                // resolver and put its signal in Ctx. Core DTOs stay handle-free.
                if let Some(object) = params.as_object_mut() {
                    object.remove("abort");
                }
                let mut options: $module::$options = options(params, &[
                    $($field,)* "maxRetries", "timeout", "providerOptions", "headers",
                ])?;
                options.abort_signal = ctx.abort;
                let output = $module::$call(model.as_ref(), options).await.map_err(OpsError::AiMux)?;
                result(output)
            })
        }
    };
}

model_call!(
    embed,
    Embedding,
    embedding_model,
    EmbeddingCallOptions,
    embed,
    ["values"]
);
model_call!(
    generate_image,
    Image,
    image_model,
    ImageCallOptions,
    generate_image,
    [
        "prompt",
        "n",
        "size",
        "aspectRatio",
        "seed",
        "files",
        "mask"
    ]
);
model_call!(
    generate_speech,
    Speech,
    speech_model,
    SpeechCallOptions,
    generate_speech,
    [
        "text",
        "voice",
        "outputFormat",
        "instructions",
        "speed",
        "language"
    ]
);
model_call!(
    transcribe,
    Transcription,
    transcription_model,
    TranscriptionCallOptions,
    transcribe,
    ["audio", "mediaType"]
);
model_call!(
    rerank,
    Reranking,
    reranking_model,
    RerankingCallOptions,
    rerank,
    ["query", "documents", "topN"]
);
model_call!(
    generate_video,
    Video,
    video_model,
    VideoCallOptions,
    generate_video,
    [
        "prompt",
        "n",
        "aspectRatio",
        "resolution",
        "duration",
        "fps",
        "seed",
        "image",
        "frameImages",
        "inputReferences",
        "generateAudio",
        "poll"
    ]
);
model_call!(
    search,
    Search,
    search_model,
    SearchCallOptions,
    search,
    [
        "query",
        "maxResults",
        "includeRawContent",
        "timeRange",
        "includeDomains",
        "excludeDomains"
    ]
);

#[cfg(test)]
mod tests {
    use super::*;
    use aimux_core::{AbortSignal, error::AiMuxError};
    use serde_json::json;
    use std::{
        future::Future,
        pin::Pin,
        sync::{Arc, Mutex},
    };

    struct MockModel {
        output: Value,
        calls: Mutex<Vec<Value>>,
        failure: Option<AiMuxError>,
    }

    impl MockModel {
        fn new(output: Value) -> Arc<Self> {
            Arc::new(Self {
                output,
                calls: Mutex::new(Vec::new()),
                failure: None,
            })
        }

        fn respond<T: DeserializeOwned>(
            &self,
            options: impl Serialize,
            abort: &Option<AbortSignal>,
        ) -> Result<T, AiMuxError> {
            assert!(
                abort.is_some(),
                "the context's abort signal must reach the model"
            );
            self.calls
                .lock()
                .unwrap()
                .push(serde_json::to_value(options).unwrap());
            if let Some(error) = &self.failure {
                return Err(error.clone());
            }
            Ok(serde_json::from_value(self.output.clone()).unwrap())
        }
    }

    macro_rules! identity {
        () => {
            fn provider(&self) -> &str {
                "mock"
            }
            fn model_id(&self) -> &str {
                "mock-model"
            }
        };
    }

    // Spell out async-trait's boxed future signature so tests need no extra
    // dependencies beyond the ops crate's contracted dependencies.
    macro_rules! mock_method {
        ($method:ident, $module:ident, $options:ident, $result:ident) => {
            fn $method<'life0, 'life1, 'async_trait>(
                &'life0 self,
                options: &'life1 $module::$options,
            ) -> Pin<
                Box<
                    dyn Future<Output = Result<$module::$result, AiMuxError>> + Send + 'async_trait,
                >,
            >
            where
                'life0: 'async_trait,
                'life1: 'async_trait,
                Self: 'async_trait,
            {
                Box::pin(async move { self.respond(options, &options.abort_signal) })
            }
        };
    }

    impl embedding_model::EmbeddingModel for MockModel {
        identity!();
        fn max_embeddings_per_call(&self) -> Option<u32> {
            None
        }
        fn supports_parallel_calls(&self) -> bool {
            true
        }
        mock_method!(
            do_embed,
            embedding_model,
            EmbeddingCallOptions,
            EmbeddingResult
        );
    }
    impl image_model::ImageModel for MockModel {
        identity!();
        fn max_images_per_call(&self) -> Option<u32> {
            None
        }
        mock_method!(do_generate, image_model, ImageCallOptions, ImageResult);
    }
    impl speech_model::SpeechModel for MockModel {
        identity!();
        mock_method!(do_generate, speech_model, SpeechCallOptions, SpeechResult);
    }
    impl transcription_model::TranscriptionModel for MockModel {
        identity!();
        mock_method!(
            do_generate,
            transcription_model,
            TranscriptionCallOptions,
            TranscriptionResult
        );
    }
    impl reranking_model::RerankingModel for MockModel {
        identity!();
        mock_method!(
            do_rerank,
            reranking_model,
            RerankingCallOptions,
            RerankingResult
        );
    }
    impl search_model::SearchModel for MockModel {
        identity!();
        mock_method!(do_search, search_model, SearchCallOptions, SearchResult);
    }
    impl video_model::VideoModel for MockModel {
        identity!();
        fn max_videos_per_call(&self) -> Option<u32> {
            Some(1)
        }
        fn do_start<'life0, 'life1, 'async_trait>(
            &'life0 self,
            options: &'life1 video_model::VideoCallOptions,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<video_model::VideoOperationStart, AiMuxError>>
                    + Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                assert!(options.abort_signal.is_some());
                self.calls
                    .lock()
                    .unwrap()
                    .push(serde_json::to_value(options).unwrap());
                Ok(video_model::VideoOperationStart {
                    operation: json!({"id": "task"}),
                    warnings: vec![],
                    provider_metadata: None,
                    response: Default::default(),
                })
            })
        }
        fn do_status<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            operation: &'life1 Value,
            options: &'life2 video_model::VideoCallOptions,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<video_model::VideoOperationStatus, AiMuxError>>
                    + Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                assert_eq!(operation, &json!({"id": "task"}));
                self.respond(options, &options.abort_signal)
                    .map(video_model::VideoOperationStatus::Completed)
            })
        }
    }

    fn context() -> Ctx {
        Ctx {
            owner: crate::handle::Owner::CProcess,
            abort: Some(AbortSignal::new()),
        }
    }

    async fn invoke(op: &str, entry: Entry, params: Value) -> Result<Value, OpsError> {
        let mut registered = vec![];
        register(&mut registered);
        let registry = crate::handle::HandleRegistry::default();
        let ctx = context();
        let handle = registry.insert(ctx.owner, entry).unwrap();
        crate::dispatch::Dispatcher::new(&registry, &registered)
            .call(ctx, handle, op, params)
            .await
    }

    fn controls(mut params: Value) -> Value {
        params["maxRetries"] = json!(0);
        params["timeout"] = json!({"totalMs": 10000});
        params["headers"] = json!({"x-test": "preserved"});
        params["providerOptions"] = json!({"mock": {"arbitrary": {"nested": [null, 1, "值"]}}});
        params
    }

    macro_rules! round_trip {
        ($test:ident, $op:literal, $kind:ident, $params:expr, $output:expr) => {
            #[tokio::test]
            async fn $test() {
                let params = controls($params);
                let expected = $output;
                let model = MockModel::new(expected.clone());
                let output = invoke($op, Entry::$kind(model.clone()), params.clone())
                    .await
                    .unwrap();
                assert_eq!(output, expected);
                assert_eq!(*model.calls.lock().unwrap(), vec![params]);
            }
        };
    }

    round_trip!(
        embedding_single,
        "embedding.embed",
        Embedding,
        json!({"values": ["hello"]}),
        json!({"embeddings": [[1.0, 2.0]], "usage": {"tokens": 1}, "warnings": []})
    );
    round_trip!(
        embedding_many_preserves_all_inputs,
        "embedding.embedMany",
        Embedding,
        json!({"values": ["first", "第二", "🚀"]}),
        json!({"embeddings": [[1.0], [2.0], [3.0]], "warnings": []})
    );
    round_trip!(
        embedding_many_preserves_empty_input,
        "embedding.embedMany",
        Embedding,
        json!({"values": []}),
        json!({"embeddings": [], "warnings": []})
    );
    round_trip!(
        image_generation,
        "image.generate",
        Image,
        json!({"prompt": "scene", "n": 2, "size": "256x256", "aspectRatio": "1:1", "seed": 7,
            "files": [{"type": "file", "mediaType": "image/png", "data": "AAH/"}],
            "mask": {"type": "url", "url": "https://example.test/mask.png"}}),
        json!({"images": ["AAH/", "AgM="], "warnings": [], "response": {"modelId": "image"}})
    );
    round_trip!(
        speech_generation,
        "speech.generate",
        Speech,
        json!({"text": "你好", "voice": "voice", "outputFormat": "wav", "speed": 1.5, "language": "zh", "instructions": "slowly"}),
        json!({"audio": "AAH/", "warnings": [], "usage": null, "response": {"body": null}})
    );
    round_trip!(
        transcription_base64_bytes,
        "transcription.generate",
        Transcription,
        json!({"audio": "AAH/", "mediaType": "audio/wav"}),
        json!({"text": "你好", "segments": [{"text": "你好", "startSecond": 0.0, "endSecond": 1.5}],
            "language": "zh", "durationInSeconds": 1.5, "warnings": [], "response": {}})
    );
    round_trip!(
        reranking_text_documents,
        "reranking.rerank",
        Reranking,
        json!({"query": "q", "documents": {"type": "text", "values": ["a", "b"]}, "topN": 1}),
        json!({"ranking": [{"index": 1, "relevanceScore": 0.75}], "warnings": []})
    );
    round_trip!(
        reranking_object_documents,
        "reranking.rerank",
        Reranking,
        json!({"query": "q", "documents": {"type": "object", "values": [{"arbitrary": [null, 1]}]}}),
        json!({"ranking": [{"index": 0, "relevanceScore": 1.0}]})
    );
    round_trip!(
        search_extension,
        "search.search",
        Search,
        json!({"query": "q", "maxResults": 2, "includeRawContent": true, "timeRange": "week",
            "includeDomains": ["example.test"], "excludeDomains": ["excluded.test"]}),
        json!({"results": [{"title": "hit", "url": "https://example.test", "content": "snippet", "rawContent": "raw", "score": 0.5}],
            "answer": "answer", "warnings": [], "response": {"body": null}, "providerMetadata": {"mock": {"extra": true}}})
    );

    #[tokio::test]
    async fn video_uses_core_batching_and_polling() {
        let expected = json!({"videos": [{"type": "base64", "data": "AAH/", "mediaType": "video/mp4"}],
            "warnings": [], "response": {}});
        let model = MockModel::new(expected.clone());
        let params = controls(
            json!({"prompt": "scene", "n": 2, "poll": {"intervalMs": 0, "timeoutMs": 1000},
            "resolution": "1280x720", "duration": 3, "fps": 24, "generateAudio": true}),
        );
        let output = invoke(
            "video.generate",
            Entry::Video(model.clone()),
            params.clone(),
        )
        .await
        .unwrap();
        assert_eq!(
            output["videos"],
            json!([expected["videos"][0], expected["videos"][0]])
        );
        let calls = model.calls.lock().unwrap();
        assert_eq!(
            calls.len(),
            4,
            "one start and one status per provider-sized batch"
        );
        for call in calls.iter() {
            assert_eq!(call["n"], 1);
            for field in [
                "prompt",
                "poll",
                "resolution",
                "duration",
                "fps",
                "generateAudio",
                "maxRetries",
                "timeout",
                "providerOptions",
            ] {
                assert_eq!(call[field], params[field]);
            }
            assert_eq!(call["headers"]["x-test"], "preserved");
        }
    }

    #[tokio::test]
    async fn rejects_unknown_controls_and_invalid_wire_without_calling_model() {
        let model = MockModel::new(json!({}));
        for (op, entry, params) in [
            (
                "embedding.embedMany",
                Entry::Embedding(model.clone()),
                json!({"values": ["a"], "unknown": true}),
            ),
            (
                "image.generate",
                Entry::Image(model.clone()),
                json!({"n": 1, "providerOptions": {}, "callback": {}}),
            ),
            (
                "speech.generate",
                Entry::Speech(model.clone()),
                json!({"text": "x", "abortSignal": "7"}),
            ),
            (
                "transcription.generate",
                Entry::Transcription(model.clone()),
                json!({"audio": [0, 1, 255], "mediaType": "audio/wav"}),
            ),
            (
                "transcription.generate",
                Entry::Transcription(model.clone()),
                json!({"audio": {"Binary": [0]}, "mediaType": "audio/wav"}),
            ),
            (
                "reranking.rerank",
                Entry::Reranking(model.clone()),
                json!({"query": "q", "documents": []}),
            ),
            (
                "video.generate",
                Entry::Video(model.clone()),
                json!({"n": 1, "providerOptions": {}, "download": {}}),
            ),
            (
                "search.search",
                Entry::Search(model.clone()),
                json!({"query": "q", "typo": 1}),
            ),
            (
                "search.search",
                Entry::Search(model.clone()),
                json!({"query": "q", "timeout": {"totalMs": 1000, "unknown": 1}}),
            ),
            (
                "video.generate",
                Entry::Video(model.clone()),
                json!({"n": 1, "providerOptions": {}, "poll": {"intervalMs": 0, "unknown": 1}}),
            ),
            ("search.search", Entry::Search(model.clone()), json!(null)),
            (
                "embedding.embed",
                Entry::Embedding(model.clone()),
                json!({"values": "not an array"}),
            ),
        ] {
            assert!(
                matches!(
                    &invoke(op, entry, params).await.unwrap_err(),
                    OpsError::Protocol(ProtocolError::InvalidParams(_))
                ),
                "{op}"
            );
        }
        assert!(model.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn wrong_handle_retains_expected_and_actual_kinds() {
        let model = MockModel::new(json!({}));
        let error = invoke(
            "embedding.embed",
            Entry::Search(model),
            json!({"values": ["a"]}),
        )
        .await;
        let error = error.unwrap_err();
        let OpsError::Binding(BindingError::InvalidHandle { expected, actual }) = &error else {
            panic!("expected InvalidHandle");
        };
        assert_eq!(expected, &[HandleKind::Embedding]);
        assert_eq!(*actual, Some(HandleKind::Search));
    }

    #[tokio::test]
    async fn core_error_is_preserved() {
        let model = Arc::new(MockModel {
            output: Value::Null,
            calls: Mutex::new(vec![]),
            failure: Some(AiMuxError::invalid_argument("provider rejected query")),
        });
        let error = invoke(
            "search.search",
            Entry::Search(model.clone()),
            controls(json!({"query": "q"})),
        )
        .await;
        assert!(
            matches!(&error.unwrap_err(), OpsError::AiMux(AiMuxError::InvalidArgument { message, .. }) if message == "provider rejected query")
        );
        assert_eq!(model.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn context_abort_is_observed_by_core() {
        let model = MockModel::new(json!({}));
        let ctx = context();
        ctx.abort.as_ref().unwrap().abort();
        let error = search(
            ctx,
            Target::Handle(Entry::Search(model.clone())),
            json!({"query": "q"}),
        )
        .await;
        assert!(matches!(
            error,
            Err(OpsError::AiMux(AiMuxError::Aborted(_)))
        ));
        assert!(model.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn binary_results_use_v4_base64_wire() {
        let speech = speech_model::SpeechResult {
            audio: speech_model::AudioData::Binary(vec![0, 1, 255]),
            warnings: vec![],
            usage: None,
            request: None,
            response: Default::default(),
            provider_metadata: None,
        };
        assert_eq!(result(speech).unwrap()["audio"], "AAH/");
        let image = image_model::ImageResult {
            images: image_model::ImageOutputs::Binary(vec![vec![0, 1, 255]]),
            warnings: vec![],
            response: Default::default(),
            provider_metadata: None,
            usage: None,
        };
        assert_eq!(result(image).unwrap()["images"], json!(["AAH/"]));
        let options: transcription_model::TranscriptionCallOptions = options(
            json!({"audio": "AAH/", "mediaType": "audio/wav"}),
            &["audio", "mediaType"],
        )
        .unwrap();
        let transcription_model::AudioInput::Base64(audio) = options.audio else {
            panic!("expected base64");
        };
        assert_eq!(
            aimux_core::wire::decode_base64(&audio).unwrap(),
            [0, 1, 255]
        );
    }
}
