# aimux · Rust API

> Unified LLM service access layer — one API to access AI providers

This is the core implementation language. Shared reference — parameter tables,
result shapes, factory functions, and the feature coverage matrix — lives in
the [API overview](../API.md).

## Quick Start

```rust
use aimux_core::prelude::*;
use aimux_providers::{OpenAIProviderSettings, create_openai};

#[tokio::main]
async fn main() -> Result<(), AiMuxError> {
    let provider = create_openai(OpenAIProviderSettings {
        api_key: Some("sk-...".to_string()),
        ..Default::default()
    })?;
    let model = provider.chat("gpt-4o");
    let result = generate_text(&model, "What is Rust?", GenerateTextOptions::default()).await?;
    println!("{}", result.text);
    Ok(())
}
```

## Providers

Provider packages follow the AI SDK shape: `XxxProviderSettings`,
`create_xxx(settings)` and a default instance `xxx()` where the package
provides one. Required fields follow the package: OpenAI-compatible settings
require `name` and `base_url`. Models are taken from the provider (`provider.chat(id)`,
`provider.language_model(id)?`, `provider.embedding_model(id)?`, …). Model
identity follows the package (`openai.chat`, `anthropic.messages`); custom
name handling is package-specific.

```rust
use aimux_providers::anthropic::{AnthropicProviderSettings, create_anthropic};
use aimux_providers::openai::openai;

// Default instance: OPENAI_API_KEY is read when a request is made; a missing
// key fails that call with `AiMuxError::LoadApiKey`, not the constructor.
let model = openai().chat("gpt-4o");

// Explicit settings. `api_key: Some(..)` is used as given (`""` included).
let anthropic = create_anthropic(AnthropicProviderSettings {
    api_key: Some("sk-ant-...".to_string()),
    base_url: Some("https://relay.example/v1".into()),
    ..Default::default()
})?;
let model = anthropic.messages("claude-sonnet-4-5");
```

Settings are package-specific. OpenAI, Anthropic and Google take
`api_key: Option<String>` and `headers: Option<HeaderMapOpt>`; these are fixed
values, not callbacks. `fetch: Option<FetchFunction>` supplies a custom
transport. These packages have no `transform_request_body` setting;
OpenAI-compatible settings expose that hook, plus `supported_urls` and
`convert_usage` callbacks for chat models. Cohere and Mistral expose
`generate_id` for generated ids. Settings only expose `name` where the upstream
package does. Every request includes the package user-agent suffix
`ai-sdk-<package>/<version>`.
OpenAI defaults to Responses through `call` and `language_model`; `chat`
selects Chat Completions explicitly.
Retry is not a provider setting: `max_retries` on the call options (default
2, 2000 ms initial delay, factor 2).

The 281 OpenAI-compatible presets use a runtime table parsed once from
`provider_registry.json`; there is no `family` field or generated presets source.
Groq and DeepSeek are vendor packages with their own chat models. xAI and
Hugging Face use the Responses API for language models, with no chat-completions
accessor.

By-name creation covers both vendor packages and preset rows:

```rust
use aimux_providers::{create_provider, default_providers, provider_names, PresetSettings};
use aimux_core::provider_registry::create_provider_registry;

// The vendor's environment key is read when a request is made.
let provider = create_provider("groq", PresetSettings::default())?;
let model = provider.language_model("model-id")?;

// Explicit key and base URL override.
let provider = create_provider("groq", PresetSettings {
    api_key: Some("sk-...".to_string()),
    base_url: Some("https://relay.example/v1".into()),
    ..Default::default()
})?;

// Combined ids use "provider:model" (":" is the default separator).
let registry = create_provider_registry(default_providers(), Default::default());
let model = registry.language_model("groq:model-id")?;
let names = provider_names();

// Model listing is optional: providers without discovery return None.
if let Some(discovery) = provider.discovery() {
    let models = discovery.list_models().await?;
}
```

Unknown names fail with `AiMuxError::NoSuchProvider { provider_id, model_id,
model_type, available_providers }`.
Custom endpoints use `PresetSettings::base_url` or the vendor's settings.
The bindings retain their compatibility API in `aimux_providers::provider`;
its helpers and option types are not re-exported from the Rust crate root.

## Middleware

`wrap_language_model(model, middleware, model_id, provider_id)` applies the
first middleware as the outermost wrapper. Hooks can override provider id,
model id and supported URLs, transform call parameters, and wrap generation
or streaming with callbacks for both operations. Explicit id arguments take
precedence over middleware overrides. `wrap_image_model` provides the image
model hooks and the same explicit id overrides.

## Text Generation

Non-streaming text generation; returns the complete result.

```rust
let result = generate_text(
    &model,
    "Explain Rust ownership.",
    GenerateTextOptions {
        max_output_tokens: Some(100),
        temperature: Some(0.7),
        max_retries: Some(0), // disable retries for this call
        timeout: Some(aimux_core::options::TimeoutConfiguration {
            total_ms: Some(30_000),
            first_chunk_ms: Some(5_000),
            chunk_ms: Some(2_000),
        }),
        ..Default::default()
    },
).await?;
```

Cancellation — set the runtime `abort_signal` handle (never crosses the JSON
boundary; FFI bindings cannot set it):

```rust
let signal = aimux_core::AbortSignal::new();
let opts = GenerateTextOptions {
    abort_signal: Some(signal.clone()),
    ..Default::default()
};
let task = tokio::spawn(generate_text(&model, "Explain Rust.", opts));
signal.abort(); // cancels connect, body reads, and streaming
let result = task.await.unwrap()?;
```

> Parameters, return value, and the `raw.content` variants are documented in
> the [API overview](../API.md#text-generation).

## Streaming Generation

Returns generated content as `TextStreamPart` items, output chunk by chunk.
Provider-layer `StreamPart::ResponseMetadata` events are consumed by the call
layer; they are not emitted by `stream_text`.

```rust
use futures::StreamExt;

let result = stream_text(&model, "Write a haiku.", GenerateTextOptions::default()).await?;
let mut stream = result.stream;
while let Some(part) = stream.next().await {
    match part? {
        TextStreamPart::TextDelta { delta, .. } => print!("{}", delta),
        TextStreamPart::Finish { .. } => println!("\n[done]"),
        _ => {}
    }
}
```

Streaming honors the same `timeout` / `abort_signal` options as
[text generation](#text-generation); streamed timeouts surface as an
`Err(AiMuxError::Timeout(..))` stream item, and aborting the signal ends the
stream with `Err(AiMuxError::Aborted(..))`. Provider-reported error events
remain `Ok(TextStreamPart::Error { .. })` data.

> Stream part variants are documented in the [API overview](../API.md#streaming-generation).

## Tool Calling

Tool definitions are language-agnostic data descriptions (JSON Schema) that require no macros.

### Defining Tools

```rust
use aimux_core::tool::FunctionTool;
use serde_json::json;

let tool = FunctionTool::new("get_weather", json!({
    "type": "object",
    "properties": {
        "location": { "type": "string" }
    },
    "required": ["location"]
}));
```

### Tool Selection Strategy

Set `tool_choice` on `GenerateTextOptions` (`ToolChoice::Auto` / `None` /
`Required` / `Tool { tool_name: "get_weather".into() }`).

## Multi-Role Messages

`prompt` accepts a message array to implement multi-turn conversation; roles support `system` / `user` / `assistant` / `tool`:

```rust
// Rust — tool round-trip
let messages = vec![
    ModelMessage::user("What's the weather in Tokyo?"),
    ModelMessage {
        role: Role::Assistant,
        content: MessageContent::Parts(vec![ContentPart::tool_call(
            "call_abc", "get_weather", json!({"location": "Tokyo"}),
        )]),
    },
    ModelMessage {
        role: Role::Tool,
        content: MessageContent::Parts(vec![ContentPart::tool_result(
            "call_abc", json!({"temperature": 22, "condition": "sunny"}),
        )]),
    },
];
let result = generate_text(&model, messages, opts).await?;
```

## Vector Embedding

Converts text into a vector representation.

```rust
use aimux_core::embedding_model::{EmbeddingCallOptions, embed};

let model = provider.embedding("text-embedding-3-small");
let opts = EmbeddingCallOptions::new("hello");
let result = embed(&model, opts).await?;
// result.embeddings[0] is Vec<f32>
```

## Speech Synthesis (TTS)

Converts text into speech audio.

```rust
use aimux_core::speech_model::{SpeechCallOptions, generate_speech};

let model = provider.speech("tts-1");
let opts = SpeechCallOptions::new("Hello world!");
let result = generate_speech(&model, opts).await?;
// result.audio is AudioData::Base64(String) or AudioData::Binary(Vec<u8>)
```

## Speech to Text (STT)

Converts audio into text (non-streaming).

```rust
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, transcribe};

let model = provider.transcription("whisper-1");
let opts = TranscriptionCallOptions::new(
    AudioInput::Base64(audio_base64),
    "audio/mp3",
);
let result = transcribe(&model, opts).await?;
// result.text, result.segments, result.language
```

## Image Generation

```rust
use aimux_core::image_model::{ImageCallOptions, generate_image};

let model = provider.image("dall-e-3");
let opts = ImageCallOptions { prompt: Some("A cute sea otter".into()), n: 1, .. };
let result = generate_image(&model, opts).await?;
// result.images is ImageOutputs::Base64(Vec<String>) or Binary(Vec<Vec<u8>>)
```

## Video Generation

Video generation typically returns a URL (not binary).

```rust
use aimux_core::video_model::{VideoCallOptions, generate_video};

let model = provider.video("veo-3.0");
let opts = VideoCallOptions { prompt: Some("A cat".into()), n: 1, .. };
let result = generate_video(&model, opts).await?;
// result.videos[0] is VideoData::Url { url, media_type }
```

## Reranking

Reorders a document list by relevance.

```rust
use aimux_core::reranking_model::{RerankingCallOptions, RerankingDocuments, rerank};

let model = provider.reranking("rerank-v3.0");
let opts = RerankingCallOptions::new("What is Rust?", docs);
let result = rerank(&model, opts).await?;
// result.ranking sorted by score
```

## Search

Calls a search provider to obtain results.

```rust
use aimux_core::search_model::{SearchCallOptions, search};

let model = aimux_providers::tavily().search_model();
let opts = SearchCallOptions::new("What is Rust?");
let result = search(&model, opts).await?;
// result.results is Vec<SearchResultItem>
```

## File Upload

Uploads a file to the provider and returns a file ID.

```rust
use aimux_core::files_model::{Files, UploadFileCallOptions, UploadFileData};
use aimux_core::shared::FileBytes;

let files = provider.files();
let opts = UploadFileCallOptions::new(
    UploadFileData::Data { data: FileBytes::Base64(file_b64) },
    "application/pdf",
);
let result = files.upload_file(opts).await?;
// result.provider_reference is HashMap<String, String>
```

## Core Traits

The Rust core provides these traits/interfaces, implemented by each provider as needed:

| Trait | Method | Semantics |
|-------|------|------|
| `Provider` | `language_model`, `embedding_model`, `image_model`, `discovery` | Provider factory — holds API config, creates `LanguageModel` instances by model name |
| `ProviderDiscovery` | `list_models` | Optional model listing through `Provider::discovery()` |
| `LanguageModel` | `do_generate`, `do_stream` | Text generation |
| `EmbeddingModel` | `do_embed` | Vector embedding |
| `SpeechModel` | `do_generate` | Speech synthesis |
| `TranscriptionModel` | `do_generate`, `do_stream` | Speech to text |
| `ImageModel` | `do_generate` | Image generation |
| `RerankingModel` | `do_rerank` | Reranking |
| `VideoModel` | `do_start`, `do_status` | Video generation (Core-driven start/poll flow) |
| `SearchModel` | `do_search` | Search |
| `Files` | `upload_file` | File upload |

The user-facing API consists of the `generate_text()` / `stream_text()` free functions, which internally call the trait methods.

Concrete provider accessors such as `embedding`, `speech`, `image`,
`transcription`, `reranking` and `video` are inherent methods on the vendors
that offer them. Through `Provider`, required model methods return `Result`;
optional model methods return `Option<Result<…>>`, and `files()` returns
`Option<Arc<dyn Files>>`.

## Types

Rust types are the canonical definitions — one module per feature in
`aimux-core/src/`, re-exported through `aimux_core::prelude`:

| Module | Key types |
|------|------|
| `generate` | `generate_text` / `stream_text` functions, `GenerateResult` |
| `language_model` | `LanguageModel`, `GenerateTextResult` |
| `stream_part` | `StreamPart` (provider layer), `TextStreamPart` (call layer) |
| `options` | `GenerateTextOptions`, `CallOptions`, `ResponseFormat`, `ToolChoice`, `ReasoningEffort` |
| `message` / `language_model_message` | `ModelMessage`, `ModelPrompt`, `MessageContent`, `Role` |
| `content` | `ContentPart` (Text / Image / File / Reasoning / ToolCall / ToolResult) |
| `tool` | `Tool`, `FunctionTool`, `ProviderTool`, `ToolCall`, `ToolResult` |
| `types` | `Usage`, `TokenUsage`, `FinishReason`, `ResponseMetadata`, `Warning` |
| `shared` | `FileBytes`, `FileData`, `Size`, `AspectRatio`, `ResponseInfo` |
| `abort_signal` | `AbortSignal` |
| `embedding_model` | `EmbeddingModel`, `EmbeddingCallOptions`, `EmbeddingResult` |
| `speech_model` | `SpeechModel`, `SpeechCallOptions`, `SpeechResult` |
| `transcription_model` | `TranscriptionModel`, `TranscriptionCallOptions`, `TranscriptionResult`, `AudioInput` |
| `image_model` | `ImageModel`, `ImageCallOptions`, `ImageResult`, `ImageOutputs` |
| `video_model` | `VideoModel`, `VideoCallOptions`, `VideoResult` |
| `reranking_model` | `RerankingModel`, `RerankingCallOptions`, `RerankingResult` |
| `search_model` | `SearchModel`, `SearchCallOptions`, `SearchResult` |
| `files_model` | `Files`, `UploadFileCallOptions`, `UploadFileResult` |
| `error` | `AiMuxError` |

## Logging (RFC-0014)

aimux ships a unified `tracing`-based logging layer. It is **off by default**
(only retries and failures are logged at the default `warn` level) and never
overrides a subscriber the host application installed itself.

### Env vars (zero-code setup)

| Env var | Effect |
|---------|--------|
| `AIMUX_LOG` | RUST_LOG-style directives, e.g. `AIMUX_LOG=aimux=debug,aimux_provider_utils::http=trace` |
| `AIMUX_LOG_LEVEL` | Simple level name: `off` / `error` / `warn` / `info` / `debug` / `trace` |
| `AIMUX_LOG_BODY` | `=1` to also log request/response bodies (requires `trace` level; bodies are truncated to 4KB and auth fields are redacted) |

The subscriber is lazily registered on the first HTTP call, only when an
`AIMUX_LOG*` var is present and no global subscriber exists yet. Logs go to
stderr.

### Programmatic API

```rust
use aimux_providers::init_logging;

init_logging("debug"); // idempotent; no-op if a global subscriber exists
```

### C ABI

```c
aimux_init_logging("debug"); // aimux-ffi.h, idempotent, logs to stderr
```

### Per-language entry points

Every binding exposes the same idempotent `init_logging(level)` entry:

| Language | Entry |
|----------|-------|
| Rust | `aimux_providers::init_logging("debug")` |
| C | `aimux_init_logging("debug")` |
| Python | `aimux.init_logging("debug")` |
| Node | `initLogging("debug")` |
| Go | `aimux.InitLogging("debug")` |
| Java | `Aimux.initLogging("debug")` |
| Kotlin | `initLogging("debug")` |
| Swift | `Aimux.initLogging(level: "debug")` |
| Flutter | `initLogging("debug")` |

`level` accepts `off | error | warn | info | debug | trace` (empty/null
defaults to `warn`). All entries are no-ops when the host already registered
its own subscriber, and the `AIMUX_LOG*` env vars always take precedence.

### What is logged

- `generate` span (provider / model / modality) around every top-level call
- `http_request` span (method / host / attempt) at the HTTP throat — all
  providers share it
- `request` / `response` events at `debug`: URL **without query string**,
  body size, header **count** (values are never logged)
- `retry` (warn) / `failed` (error) events with status, attempt, delay, reason
- Stream events at `debug`: `stream_connected`, `stream_first_byte` (TTFB),
  `stream_end` (chunk count + duration)
