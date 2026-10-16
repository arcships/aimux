//! Text operations backed by Core's generation and OpenAI output paths.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use aimux_core::generate::{
    GenerateTextOptions, generate_text, generate_text_as_openai, stream_text,
};
use aimux_core::message::ModelPrompt;
use aimux_core::openai_output::{
    ChatCompletionChunk, OpenAiStreamOptions, to_chat_completion_stream,
};
use aimux_core::stream_part::TextStreamPart;
use aimux_core::{AbortSignal, AiMuxError, LanguageModel};
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::oneshot;

use crate::dispatch::{CallFuture, Ctx, EventStream, Handler, Target, decode as params};
use crate::envelope::{BindingError, OpsError};
use crate::handle::{Entry, HandleId, HandleKind};
use crate::op::{Cancellation, OpDescriptor, OpMode, RequestBytePolicy, Scope};

const LANGUAGE: &[HandleKind] = &[HandleKind::Language];

const fn descriptor(
    name: &'static str,
    mode: OpMode,
    request_type: &'static str,
    result_type: &'static str,
    event_type: Option<&'static str>,
    source_symbols: &'static [&'static str],
) -> OpDescriptor {
    OpDescriptor {
        name,
        since_ops_version: "1.0",
        mode,
        allowed_handle_kinds: LANGUAGE,
        request_type,
        result_type,
        event_type,
        scope: Scope::Current,
        terminal_type: if matches!(mode, OpMode::Stream) {
            Some("StreamFrame")
        } else {
            None
        },
        runtime_handle_fields: &[("abort", &[HandleKind::Abort])],
        required_capabilities: &[],
        cancellation: Cancellation::Abort,
        result_ownership: "owned",
        request_byte_policy: RequestBytePolicy::Json,
        side_effects: true,
        retry_owner: "core",
        source_symbols,
    }
}

pub(crate) static OPS: &[OpDescriptor] = &[
    descriptor(
        "text.generate",
        OpMode::Call,
        "aimux_ops::ops::text::TextRequest",
        "aimux_core::generate::GenerateTextResult",
        None,
        &["aimux_generate_text"],
    ),
    descriptor(
        "text.generateObject",
        OpMode::Call,
        "aimux_ops::ops::text::TextRequest",
        "aimux_core::generate::GenerateObjectResult",
        None,
        &["aimux_generate_object"],
    ),
    descriptor(
        "text.consume",
        OpMode::Call,
        "aimux_ops::ops::text::TextRequest",
        "aimux_core::generate::GenerateTextResult",
        None,
        &["aimux_consume_stream_text"],
    ),
    descriptor(
        "text.generateOpenAI",
        OpMode::Call,
        "aimux_ops::ops::text::TextRequest",
        "aimux_core::openai_output::ChatCompletion",
        None,
        &["aimux_generate_text_as_openai"],
    ),
    descriptor(
        "text.stream",
        OpMode::Stream,
        "aimux_ops::ops::text::TextRequest",
        "StreamFrame",
        Some("aimux_core::stream_part::TextStreamPart"),
        &["aimux_stream_text", "aimux_stream_text_with_abort"],
    ),
    descriptor(
        "text.streamOpenAI",
        OpMode::Stream,
        "aimux_ops::ops::text::OpenAiTextRequest",
        "StreamFrame",
        Some("aimux_core::openai_output::ChatCompletionChunk"),
        &[
            "aimux_stream_text_as_openai",
            "aimux_stream_text_as_openai_with_abort",
        ],
    ),
];

pub(crate) fn register(r: &mut Vec<(&'static OpDescriptor, Handler)>) {
    r.extend([
        (&OPS[0], Handler::Call(generate)),
        (&OPS[1], Handler::Call(generate_object)),
        (&OPS[2], Handler::Call(consume)),
        (&OPS[3], Handler::Call(generate_openai)),
        (&OPS[4], Handler::Stream(stream)),
        (&OPS[5], Handler::Stream(stream_openai)),
    ]);
}

#[derive(Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(flatten)]
    core: GenerateTextOptions,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextRequest {
    prompt: ModelPrompt,
    #[serde(default)]
    options: Options,
    #[serde(default, rename = "abort")]
    _abort: Option<HandleId>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenAiTextRequest {
    prompt: ModelPrompt,
    #[serde(default)]
    options: Options,
    #[serde(default, rename = "abort")]
    _abort: Option<HandleId>,
    #[serde(default)]
    stream_options: StreamOptions,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
struct StreamOptions {
    include_usage: bool,
    include_reasoning: bool,
}

impl Default for StreamOptions {
    fn default() -> Self {
        Self {
            include_usage: true,
            include_reasoning: true,
        }
    }
}

fn json<T: Serialize>(value: T) -> Result<Value, OpsError> {
    serde_json::to_value(value)
        .map_err(|e| OpsError::Binding(BindingError::ResultSerialization(e.to_string())))
}

fn language(target: Target) -> Result<Arc<dyn LanguageModel>, OpsError> {
    match target {
        Target::Handle(Entry::Language(model)) => Ok(model),
        other => Err(OpsError::Binding(BindingError::InvalidHandle {
            expected: LANGUAGE.to_vec(),
            actual: match other {
                Target::Global => None,
                Target::Handle(entry) => Some(entry.kind()),
            },
        })),
    }
}

impl TextRequest {
    fn lift(mut self, ctx: &Ctx) -> Self {
        // Dispatch has already resolved the optional abort handle with its registry
        // and owner policy. Retain that signal instead of looking up the ID again.
        self.options.core.abort_signal = ctx.abort.clone();
        self
    }
}

async fn cancellable<T>(
    signal: Option<AbortSignal>,
    future: impl Future<Output = Result<T, AiMuxError>>,
) -> Result<T, OpsError> {
    match signal {
        Some(signal) => tokio::select! {
            biased;
            _ = signal.cancelled() => Err(OpsError::AiMux(AiMuxError::from_abort_signal(&signal))),
            result = future => result.map_err(OpsError::AiMux),
        },
        None => future.await.map_err(OpsError::AiMux),
    }
}

fn generate(ctx: Ctx, target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let model = language(target)?;
        let request = params::<TextRequest>(value)?.lift(&ctx);
        json(
            cancellable(
                request.options.core.abort_signal.clone(),
                generate_text(model.as_ref(), request.prompt, request.options.core),
            )
            .await?,
        )
    })
}

#[allow(deprecated)]
fn generate_object(ctx: Ctx, target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let model = language(target)?;
        let request = params::<TextRequest>(value)?.lift(&ctx);
        json(
            cancellable(
                request.options.core.abort_signal.clone(),
                aimux_core::generate::generate_object(
                    model.as_ref(),
                    request.prompt,
                    request.options.core,
                ),
            )
            .await?,
        )
    })
}

fn consume(ctx: Ctx, target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let model = language(target)?;
        let request = params::<TextRequest>(value)?.lift(&ctx);
        json(
            cancellable(request.options.core.abort_signal.clone(), async move {
                stream_text(model.as_ref(), request.prompt, request.options.core)
                    .await?
                    .consume()
                    .await
            })
            .await?,
        )
    })
}

fn generate_openai(ctx: Ctx, target: Target, value: Value) -> CallFuture {
    Box::pin(async move {
        let model = language(target)?;
        let request = params::<TextRequest>(value)?.lift(&ctx);
        json(
            cancellable(
                request.options.core.abort_signal.clone(),
                generate_text_as_openai(model.as_ref(), request.prompt, request.options.core),
            )
            .await?,
        )
    })
}

fn stream(ctx: Ctx, target: Target, value: Value) -> Result<EventStream, OpsError> {
    let model = language(target)?;
    let request = params::<TextRequest>(value)?.lift(&ctx);
    Ok(start_stream(model, request, None))
}

fn stream_openai(ctx: Ctx, target: Target, value: Value) -> Result<EventStream, OpsError> {
    let model = language(target)?;
    let request: OpenAiTextRequest = params(value)?;
    let options = OpenAiStreamOptions {
        include_usage: request.stream_options.include_usage,
        include_reasoning: request.stream_options.include_reasoning,
    };
    let request = TextRequest {
        prompt: request.prompt,
        options: request.options,
        _abort: request._abort,
    }
    .lift(&ctx);
    Ok(start_stream(model, request, Some(options)))
}

fn start_stream(
    model: Arc<dyn LanguageModel>,
    request: TextRequest,
    openai: Option<OpenAiStreamOptions>,
) -> EventStream {
    let signal = request.options.core.abort_signal.clone();
    let setup = async move {
        let result = cancellable(
            signal.clone(),
            stream_text(model.as_ref(), request.prompt, request.options.core),
        )
        .await?;
        let native = NativeEvents::new(result.stream, signal);
        let events: EventStream = match openai {
            Some(options) => {
                let (sender, terminal) = oneshot::channel();
                let input = TerminalGuard {
                    native,
                    sender: Some(sender),
                };
                let converted =
                    to_chat_completion_stream(Box::pin(input), model.model_id(), options);
                Box::pin(OpenAiEvents {
                    chunks: converted.stream,
                    terminal,
                    ended: false,
                })
            }
            None => Box::pin(native.map(json_part)),
        };
        Ok(events)
    };
    Box::pin(
        futures::stream::once(setup)
            .map(|result| match result {
                Ok(events) => events,
                Err(error) => {
                    Box::pin(futures::stream::once(std::future::ready(Err(error)))) as EventStream
                }
            })
            .flatten(),
    )
}

fn json_part(part: Result<TextStreamPart, AiMuxError>) -> Result<Value, OpsError> {
    json(part.map_err(OpsError::AiMux)?)
}

type NativeStream = Pin<Box<dyn Stream<Item = Result<TextStreamPart, AiMuxError>> + Send>>;

struct NativeEvents {
    inner: NativeStream,
    cancelled: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    signal: Option<AbortSignal>,
    ended: bool,
}

impl NativeEvents {
    fn new(inner: NativeStream, signal: Option<AbortSignal>) -> Self {
        let cancelled = signal
            .as_ref()
            .map(|signal| Box::pin(signal.cancelled()) as Pin<Box<dyn Future<Output = ()> + Send>>);
        Self {
            inner,
            cancelled,
            signal,
            ended: false,
        }
    }
}

impl Stream for NativeEvents {
    type Item = Result<TextStreamPart, AiMuxError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.ended {
            return Poll::Ready(None);
        }
        if let Some(cancelled) = &mut this.cancelled
            && cancelled.as_mut().poll(cx).is_ready()
        {
            this.ended = true;
            return Poll::Ready(Some(Err(AiMuxError::Aborted("request aborted".into()))));
        }
        match this.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Err(error))) if error.is_recoverable_stream_error() => {
                Poll::Ready(Some(Ok(TextStreamPart::Error { error })))
            }
            Poll::Ready(Some(Ok(TextStreamPart::Abort { .. }))) => {
                this.ended = true;
                Poll::Ready(Some(Err(this.signal.as_ref().map_or_else(
                    || AiMuxError::Aborted("request aborted".into()),
                    AiMuxError::from_abort_signal,
                ))))
            }
            Poll::Ready(Some(Err(error))) => {
                this.ended = true;
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(None) => {
                this.ended = true;
                Poll::Ready(None)
            }
            other => other,
        }
    }
}

// Preserve runtime failures before Core's OpenAI converter turns them into chunks.
struct TerminalGuard {
    native: NativeEvents,
    sender: Option<oneshot::Sender<AiMuxError>>,
}

impl Stream for TerminalGuard {
    type Item = Result<TextStreamPart, AiMuxError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match Pin::new(&mut this.native).poll_next(cx) {
            Poll::Ready(Some(Err(error))) => {
                if let Some(sender) = this.sender.take() {
                    let _ = sender.send(error);
                }
                Poll::Ready(None)
            }
            other => other,
        }
    }
}

struct OpenAiEvents {
    chunks: Pin<Box<dyn Stream<Item = Result<ChatCompletionChunk, AiMuxError>> + Send>>,
    terminal: oneshot::Receiver<AiMuxError>,
    ended: bool,
}

impl Stream for OpenAiEvents {
    type Item = Result<Value, OpsError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.ended {
            return Poll::Ready(None);
        }
        let next = this.chunks.as_mut().poll_next(cx);
        if let Ok(error) = this.terminal.try_recv() {
            this.ended = true;
            return Poll::Ready(Some(Err(OpsError::AiMux(error))));
        }
        match next {
            Poll::Ready(Some(Ok(chunk))) => Poll::Ready(Some(json(chunk))),
            Poll::Ready(Some(Err(error))) => {
                this.ended = true;
                Poll::Ready(Some(Err(OpsError::AiMux(error))))
            }
            Poll::Ready(None) => {
                this.ended = true;
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handle::{HandleRegistry, Owner};
    use aimux_core::options::CallOptions;
    use aimux_core::result::{GenerateContent, GenerateResult, StreamResult};
    use aimux_core::stream_part::StreamPart;
    use aimux_core::types::{FinishReason, FinishReasonUnified, Usage};
    use serde_json::json;
    use std::sync::Mutex;
    use std::time::SystemTime;

    #[derive(Clone)]
    struct MockModel {
        text: String,
        parts: Vec<Result<StreamPart, AiMuxError>>,
        pending: bool,
        setup_error: bool,
        seen: Arc<Mutex<Option<CallOptions>>>,
    }

    fn finish() -> FinishReason {
        FinishReason {
            unified: FinishReasonUnified::Stop,
            raw: Some("stop".into()),
        }
    }

    impl MockModel {
        fn new(text: &str) -> Self {
            Self {
                text: text.into(),
                parts: vec![
                    Ok(StreamPart::StreamStart { warnings: vec![] }),
                    Ok(StreamPart::TextStart {
                        id: "a".into(),
                        provider_metadata: None,
                    }),
                    Ok(StreamPart::TextDelta {
                        id: "a".into(),
                        delta: text.into(),
                        provider_metadata: None,
                    }),
                    Ok(StreamPart::TextEnd {
                        id: "a".into(),
                        provider_metadata: None,
                    }),
                    Ok(StreamPart::Finish {
                        finish_reason: finish(),
                        usage: Usage::default(),
                        provider_metadata: None,
                    }),
                ],
                pending: false,
                setup_error: false,
                seen: Arc::new(Mutex::new(None)),
            }
        }
        fn entry(&self) -> Target {
            Target::Handle(Entry::Language(Arc::new(self.clone())))
        }
    }

    // Spell out async_trait's signatures so this module needs no extra test dependency.
    impl LanguageModel for MockModel {
        fn provider(&self) -> &str {
            "mock.chat"
        }
        fn model_id(&self) -> &str {
            "mock-model"
        }
        fn do_generate<'life0, 'life1, 'async_trait>(
            &'life0 self,
            options: &'life1 CallOptions,
        ) -> Pin<Box<dyn Future<Output = Result<GenerateResult, AiMuxError>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                *self.seen.lock().unwrap() = Some(options.clone());
                if self.pending {
                    return futures::future::pending().await;
                }
                Ok(GenerateResult {
                    content: vec![GenerateContent::Text {
                        text: self.text.clone(),
                        provider_metadata: None,
                    }],
                    finish_reason: finish(),
                    usage: Usage::default(),
                    warnings: vec![],
                    provider_metadata: None,
                    response: None,
                    request: None,
                })
            })
        }
        fn do_stream<'life0, 'life1, 'async_trait>(
            &'life0 self,
            options: &'life1 CallOptions,
        ) -> Pin<Box<dyn Future<Output = Result<StreamResult, AiMuxError>> + Send + 'async_trait>>
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            Self: 'async_trait,
        {
            Box::pin(async move {
                *self.seen.lock().unwrap() = Some(options.clone());
                if self.text == "setup-pending" {
                    return futures::future::pending().await;
                }
                if self.setup_error {
                    return Err(AiMuxError::Other("setup failed".into()));
                }
                let stream: Pin<Box<dyn Stream<Item = Result<StreamPart, AiMuxError>> + Send>> =
                    if self.pending {
                        Box::pin(futures::stream::pending())
                    } else {
                        Box::pin(futures::stream::iter(self.parts.clone()))
                    };
                Ok(StreamResult {
                    stream,
                    request: None,
                    response: None,
                })
            })
        }
    }

    fn ctx() -> Ctx {
        Ctx {
            owner: Owner::CProcess,
            abort: None,
            now: SystemTime::now,
        }
    }
    fn request() -> Value {
        json!({"prompt":"hello", "options":{"maxRetries":0}})
    }

    #[tokio::test]
    async fn calls_reuse_core_results_and_options() {
        let model = MockModel::new("你好 🌏");
        let result = generate(
            ctx(),
            model.entry(),
            json!({
                "prompt":[{"role":"user","content":"hi"}],
                "options":{"maxRetries":0,"maxOutputTokens":7,"temperature":0.2,
                    "providerOptions":{"custom":{"anything":{"x":1}}}}
            }),
        )
        .await
        .unwrap();
        assert_eq!(result["text"], "你好 🌏");
        assert_eq!(result["steps"].as_array().unwrap().len(), 1);
        let seen = model.seen.lock().unwrap().clone().unwrap();
        assert_eq!(seen.max_output_tokens, Some(7));
        assert_eq!(seen.temperature, Some(0.2));
        assert_eq!(seen.provider_options.unwrap()["custom"]["anything"]["x"], 1);
        let result = consume(ctx(), model.entry(), request()).await.unwrap();
        assert_eq!(result["text"], "你好 🌏");
        let result = generate_openai(ctx(), model.entry(), request())
            .await
            .unwrap();
        assert_eq!(result["object"], "chat.completion");
        assert_eq!(result["choices"][0]["message"]["content"], "你好 🌏");
    }

    #[tokio::test]
    async fn object_generation_uses_core_validation() {
        let result = generate_object(ctx(), MockModel::new(r#"{"answer":42}"#).entry(), request())
            .await
            .unwrap();
        assert_eq!(result["object"], json!({"answer":42}));
        assert!(matches!(
            generate_object(ctx(), MockModel::new("invalid JSON").entry(), request()).await,
            Err(OpsError::AiMux(_))
        ));
    }

    #[test]
    fn controls_are_strict_but_provider_options_stay_open() {
        for value in [
            json!({"prompt":"x","typo":1}),
            json!({"prompt":"x","options":{"typo":1}}),
            json!({"prompt":"x","options":{"abortSignal":{}}}),
            json!({"prompt":"x","abort":7}),
        ] {
            assert!(stream(ctx(), MockModel::new("x").entry(), value).is_err());
        }
        assert!(
            stream_openai(
                ctx(),
                MockModel::new("x").entry(),
                json!({
                    "prompt":"x","streamOptions":{"typo":true}
                })
            )
            .is_err()
        );
        assert!(
            stream(
                ctx(),
                MockModel::new("x").entry(),
                json!({
                    "prompt":"x", "options":{"providerOptions":{"custom":{"unknown":true}}}
                })
            )
            .is_ok()
        );
    }

    #[tokio::test]
    async fn native_errors_are_data_and_later_text_survives() {
        let mut model = MockModel::new("after");
        model.parts.insert(
            1,
            Ok(StreamPart::Error {
                error: AiMuxError::Other("provider warning".into()),
            }),
        );
        model
            .parts
            .insert(2, Err(AiMuxError::JsonParse("malformed frame".into())));
        let events = stream(ctx(), model.entry(), request())
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        assert!(events.iter().all(Result::is_ok));
        let events = events.into_iter().map(Result::unwrap).collect::<Vec<_>>();
        assert_eq!(events.iter().filter(|v| v["type"] == "error").count(), 2);
        assert!(
            events
                .iter()
                .any(|v| v["type"] == "text-delta" && v["text"] == "after")
        );
        assert_eq!(events.last().unwrap()["type"], "finish");
    }

    #[tokio::test]
    async fn both_streams_preserve_runtime_failure() {
        for openai in [false, true] {
            let mut model = MockModel::new("before");
            model
                .parts
                .insert(3, Err(AiMuxError::Other("connection lost".into())));
            let mut events = if openai {
                stream_openai(ctx(), model.entry(), request())
            } else {
                stream(ctx(), model.entry(), request())
            }
            .unwrap();
            let mut errors = 0;
            while let Some(event) = events.next().await {
                if let Err(OpsError::AiMux(AiMuxError::Other(message))) = event {
                    assert_eq!(message, "connection lost");
                    errors += 1;
                    assert!(events.next().await.is_none());
                    break;
                }
            }
            assert_eq!(errors, 1, "openai={openai}");
        }
    }

    #[tokio::test]
    async fn setup_failure_is_one_stream_error() {
        for openai in [false, true] {
            let mut model = MockModel::new("x");
            model.setup_error = true;
            let events = if openai {
                stream_openai(ctx(), model.entry(), request())
            } else {
                stream(ctx(), model.entry(), request())
            }
            .unwrap()
            .collect::<Vec<_>>()
            .await;
            assert_eq!(events.len(), 1);
            assert!(
                matches!(&events[0], Err(OpsError::AiMux(AiMuxError::Other(message))) if message == "setup failed")
            );
        }
    }

    #[tokio::test]
    async fn openai_stream_errors_remain_data_and_options_apply() {
        let mut model = MockModel::new("after");
        model
            .parts
            .insert(1, Err(AiMuxError::JsonParse("malformed frame".into())));
        let events = stream_openai(
            ctx(),
            model.entry(),
            json!({
                "prompt":"x", "streamOptions":{"includeUsage":false,"includeReasoning":false}
            }),
        )
        .unwrap()
        .collect::<Vec<_>>()
        .await;
        assert!(events.iter().all(Result::is_ok));
        let events = events.into_iter().map(Result::unwrap).collect::<Vec<_>>();
        assert!(
            events
                .iter()
                .all(|v| v["object"] == "chat.completion.chunk")
        );
        assert!(
            events
                .iter()
                .any(|v| v["choices"][0]["delta"]["content"] == "after")
        );
        assert!(events.iter().all(|v| v.get("usage").is_none()));
    }

    #[tokio::test]
    async fn cancellation_interrupts_setup_call_and_idle_stream() {
        let mut model = MockModel::new("x");
        model.pending = true;
        let signal = AbortSignal::new();
        let mut context = ctx();
        context.abort = Some(signal.clone());
        let pending = generate(context, model.entry(), request());
        signal.abort();
        assert!(matches!(
            pending.await,
            Err(OpsError::AiMux(AiMuxError::Aborted(_)))
        ));
        for openai in [false, true] {
            let signal = AbortSignal::new();
            let mut context = ctx();
            context.abort = Some(signal.clone());
            let mut events = if openai {
                stream_openai(context, model.entry(), request())
            } else {
                stream(context, model.entry(), request())
            }
            .unwrap();
            let cancel = async {
                tokio::task::yield_now().await;
                signal.abort();
            };
            let read = async {
                while let Some(event) = events.next().await {
                    if matches!(event, Err(OpsError::AiMux(AiMuxError::Aborted(_)))) {
                        assert!(events.next().await.is_none());
                        return;
                    }
                }
                panic!("missing cancellation");
            };
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                tokio::join!(cancel, read);
            })
            .await
            .unwrap();
        }
    }

    #[test]
    fn invalid_model_and_abort_handles_are_rejected_before_acceptance() {
        assert!(matches!(
            stream(
                ctx(),
                Target::Handle(Entry::Abort(Arc::new(AbortSignal::new()))),
                request()
            ),
            Err(OpsError::Binding(BindingError::InvalidHandle { .. }))
        ));
        for abort in ["not-a-handle", "18446744073709551616"] {
            assert!(
                stream(
                    ctx(),
                    MockModel::new("x").entry(),
                    json!({"prompt":"x","abort":abort})
                )
                .is_err()
            );
        }
    }

    #[tokio::test]
    async fn explicit_abort_handle_is_lifted_and_cloned() {
        let signal = AbortSignal::new();
        let handle = HandleRegistry::global()
            .insert(Owner::CProcess, Entry::Abort(Arc::new(signal.clone())))
            .unwrap();
        let registry = HandleRegistry::global();
        let model = registry
            .insert(
                Owner::CProcess,
                Entry::Language(Arc::new(MockModel::new("x"))),
            )
            .unwrap();
        let events = crate::dispatch::stream(
            ctx(),
            model,
            "text.stream",
            json!({"prompt":"x", "abort":handle}),
        )
        .unwrap();
        drop(registry.remove(Owner::CProcess, handle));
        drop(registry.remove(Owner::CProcess, model));
        signal.abort();
        let frames = events.collect::<Vec<_>>().await;
        assert_eq!(frames.len(), 1);
        assert_eq!(
            serde_json::to_value(&frames[0]).unwrap()["outcome"],
            "cancelled"
        );
    }

    #[tokio::test]
    async fn cancellation_interrupts_pending_provider_setup() {
        let model = MockModel::new("setup-pending");
        let signal = AbortSignal::new();
        let mut context = ctx();
        context.abort = Some(signal.clone());
        let mut events = stream_openai(context, model.entry(), request()).unwrap();
        let cancel = async {
            tokio::task::yield_now().await;
            signal.abort();
        };
        let read = async {
            assert!(matches!(
                events.next().await,
                Some(Err(OpsError::AiMux(AiMuxError::Aborted(_))))
            ));
            assert!(events.next().await.is_none());
        };
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            tokio::join!(cancel, read);
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn real_dispatch_runs_all_six_text_operations() {
        let registry = HandleRegistry::default();
        let mut registrations = vec![];
        register(&mut registrations);
        let dispatcher = crate::dispatch::Dispatcher::new(&registry, &registrations);
        let model = MockModel::new(r#"{"answer":42}"#);
        let id = registry
            .insert(ctx().owner, Entry::Language(Arc::new(model.clone())))
            .unwrap();
        for name in [
            "text.generate",
            "text.generateObject",
            "text.consume",
            "text.generateOpenAI",
        ] {
            let result = dispatcher.call(ctx(), id, name, request()).await.unwrap();
            match name {
                "text.generateObject" => assert_eq!(result["object"], json!({"answer":42})),
                "text.generateOpenAI" => {
                    assert_eq!(result["object"], "chat.completion");
                    assert_eq!(result["choices"][0]["message"]["content"], model.text);
                }
                _ => assert_eq!(result["text"], model.text),
            }
        }
        for name in ["text.stream", "text.streamOpenAI"] {
            let frames = dispatcher
                .stream(ctx(), id, name, request())
                .unwrap()
                .map(|frame| serde_json::to_value(frame).unwrap())
                .collect::<Vec<_>>()
                .await;
            assert!(frames.len() > 1);
            for (seq, frame) in frames.iter().enumerate() {
                assert_eq!(frame["seq"], seq.to_string());
            }
            assert_eq!(
                frames.iter().filter(|frame| frame["type"] == "end").count(),
                1
            );
            assert_eq!(frames.last().unwrap()["outcome"], "success");
        }
    }

    #[tokio::test]
    async fn every_text_op_rejects_foreign_unknown_and_wrong_kind_abort_before_provider() {
        let registry = HandleRegistry::default();
        let mut registrations = vec![];
        register(&mut registrations);
        let dispatcher = crate::dispatch::Dispatcher::new(&registry, &registrations);
        let owner = ctx().owner;
        let foreign = Owner::new_node().unwrap();
        let model = MockModel::new("unused");
        let id = registry
            .insert(owner, Entry::Language(Arc::new(model.clone())))
            .unwrap();
        let abort = registry
            .insert(foreign, Entry::Abort(Arc::new(AbortSignal::new())))
            .unwrap();
        for descriptor in OPS {
            let mut hidden = vec![];
            for secondary in [abort, HandleId(u64::MAX), id, HandleId(0)] {
                let value = json!({"prompt":"x", "abort":secondary});
                let error = if descriptor.mode == OpMode::Stream {
                    dispatcher
                        .stream(ctx(), id, descriptor.name, value)
                        .err()
                        .unwrap()
                } else {
                    dispatcher
                        .call(ctx(), id, descriptor.name, value)
                        .await
                        .unwrap_err()
                };
                assert!(matches!(error, OpsError::Projected(_)));
                assert_eq!(error.code(), 203);
                if secondary == abort || secondary == HandleId(u64::MAX) {
                    assert!(matches!(
                        error.unprojected(),
                        OpsError::Binding(BindingError::InvalidHandle { actual: None, .. })
                    ));
                    hidden.push(error.envelope(SystemTime::UNIX_EPOCH));
                }
            }
            assert_eq!(hidden[0], hidden[1]);
        }
        assert!(model.seen.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn isolated_dispatch_resolves_and_retains_abort_without_global_lookup() {
        let registry = HandleRegistry::default();
        let mut registrations = vec![];
        register(&mut registrations);
        let dispatcher = crate::dispatch::Dispatcher::new(&registry, &registrations);
        let model = MockModel::new("hello");
        let signal = AbortSignal::new();
        let id = registry
            .insert(ctx().owner, Entry::Language(Arc::new(model.clone())))
            .unwrap();
        let abort = registry
            .insert(ctx().owner, Entry::Abort(Arc::new(signal.clone())))
            .unwrap();
        let value = json!({"prompt":"x", "options":{"maxRetries":0}, "abort":abort});
        dispatcher
            .call(ctx(), id, "text.generate", value.clone())
            .await
            .unwrap();
        let received = model
            .seen
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .abort_signal
            .clone()
            .unwrap();
        let stream = dispatcher.stream(ctx(), id, "text.stream", value).unwrap();
        drop(registry.remove(ctx().owner, abort));
        signal.abort();
        assert!(received.is_aborted());
        let frames = stream
            .map(|frame| serde_json::to_value(frame).unwrap())
            .collect::<Vec<_>>()
            .await;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0]["outcome"], "cancelled");
        assert_eq!(frames[0]["error"]["code"], 13);
    }

    #[tokio::test]
    async fn real_text_streams_frame_recoverable_setup_and_runtime_errors() {
        let registry = HandleRegistry::default();
        let mut registrations = vec![];
        register(&mut registrations);
        let dispatcher = crate::dispatch::Dispatcher::new(&registry, &registrations);
        for name in ["text.stream", "text.streamOpenAI"] {
            for case in ["recoverable", "setup", "runtime", "large"] {
                let mut model = MockModel::new("after");
                match case {
                    "recoverable" => model
                        .parts
                        .insert(1, Err(AiMuxError::JsonParse("malformed frame".into()))),
                    "setup" => model.setup_error = true,
                    "runtime" => model
                        .parts
                        .insert(3, Err(AiMuxError::Other("connection lost".into()))),
                    "large" => model.parts.insert(
                        3,
                        Ok(StreamPart::TextDelta {
                            id: "a".into(),
                            delta: "x".repeat(crate::limits::ITEM_BYTES),
                            provider_metadata: None,
                        }),
                    ),
                    _ => unreachable!(),
                }
                let id = registry
                    .insert(ctx().owner, Entry::Language(Arc::new(model)))
                    .unwrap();
                let frames = dispatcher
                    .stream(ctx(), id, name, request())
                    .unwrap()
                    .map(|frame| serde_json::to_value(frame).unwrap())
                    .collect::<Vec<_>>()
                    .await;
                drop(registry.remove(ctx().owner, id));
                assert_eq!(
                    frames.iter().filter(|frame| frame["type"] == "end").count(),
                    1
                );
                let end = frames.last().unwrap();
                if case == "recoverable" {
                    assert_eq!(end["outcome"], "success");
                    assert!(frames.iter().any(|frame| frame["event"]["text"] == "after"
                        || frame["event"]["choices"][0]["delta"]["content"] == "after"));
                } else {
                    assert_eq!(end["outcome"], "error", "{name}: {case}");
                    assert_eq!(end["error"]["code"], if case == "large" { 305 } else { 1 });
                    assert_eq!(
                        end["error"]["kind"],
                        if case == "large" { "protocol" } else { "aimux" }
                    );
                }
            }
        }
    }

    fn fixed_now() -> SystemTime {
        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000)
    }

    #[tokio::test]
    async fn text_stream_terminal_caches_retry_after_at_dispatch_boundary() {
        let registry = HandleRegistry::default();
        let mut registrations = vec![];
        register(&mut registrations);
        let dispatcher = crate::dispatch::Dispatcher::new(&registry, &registrations);
        for name in ["text.stream", "text.streamOpenAI"] {
            let mut api = aimux_core::error::ApiCallError::new(
                "retry later",
                "https://mock.invalid",
                json!({}),
            );
            api.response_headers = Some(std::collections::HashMap::from([(
                "Retry-After".into(),
                "Tue, 14 Nov 2023 22:13:22 GMT".into(),
            )]));
            let mut model = MockModel::new("before");
            model
                .parts
                .insert(3, Err(AiMuxError::ApiCall(Box::new(api))));
            let id = registry
                .insert(ctx().owner, Entry::Language(Arc::new(model)))
                .unwrap();
            let mut context = ctx();
            context.now = fixed_now;
            let frames = dispatcher
                .stream(context, id, name, request())
                .unwrap()
                .map(|frame| serde_json::to_value(frame).unwrap())
                .collect::<Vec<_>>()
                .await;
            assert_eq!(frames.last().unwrap()["error"]["code"], 11);
            assert_eq!(frames.last().unwrap()["error"]["retryAfterMs"], 2000);
        }
    }
}
