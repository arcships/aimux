# aimux · Rust API

> Unified LLM service access layer — one API to access 325 AI providers

This is the core implementation language. Shared reference — parameter tables,
result shapes, factory functions, and the feature coverage matrix — lives in
the [API overview](../API.md).

## Quick Start

```rust
use aimux_core::prelude::*;
use aimux_providers::{OpenAIConfig, OpenAIProvider};

#[tokio::main]
async fn main() -> Result<(), AiMuxError> {
    let provider = OpenAIProvider::new(OpenAIConfig::new("sk-..."));
    let model = provider.model("gpt-4o");
    let result = generate_text(&model, "What is Rust?", GenerateTextOptions::default()).await?;
    println!("{}", result.text);
    Ok(())
}
```

## Providers

All 250 built-in OpenAI-compatible providers are registry-backed: no per-provider
`XxxConfig`/`XxxProvider` types. Look them up by name (or by the `ProviderName` enum):

> **Scope:** `provider(name)` covers only the 250 registry OpenAI-compatible
> providers; Anthropic/Google/multimodal/local → typed factories
> (`AnthropicProvider::new(..)`); custom endpoints →
> `ProviderOptions.base_url` / `OpenAIConfig::with_base_url`.
> Full list: [providers.md](providers.md).

```rust
use aimux_providers::{provider, provider_from_env, ProviderName, ProviderOptions};

// Key from the provider's env var (GROQ_API_KEY etc.), base URL & profile from
// the registry — replaces the retired `XxxConfig::from_env()`.
let model = provider_from_env("groq", "llama-3.3-70b", None)?;

// Explicit key + overrides — replaces the retired `XxxConfig::new().with_base_url(..)`.
let model = provider(
    "groq",
    Some("sk-...".to_string()),
    "llama-3.3-70b",
    Some(ProviderOptions { base_url: Some("https://relay.example/v1".into()), ..Default::default() }),
)?;

// Typed name (推荐——IDE-completable, compile-checked):
let model = provider(ProviderName::Groq, None, "llama-3.3-70b", None)?;
// 字符串形式同样可用（弱类型兼容）:
let model = provider_from_env("groq", "llama-3.3-70b", None)?;
```

`ProviderName` is generated from `provider_registry.json` (250 variants, `as_str`/
`from_str`/`ALL`). Unknown names fail with `AiMuxError::NoSuchProvider { provider_id }`.

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
let signal = aimux_core::shared::AbortSignal::new();
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

Returns generated content as a stream, output chunk by chunk.

```rust
use futures::StreamExt;

let result = stream_text(&model, "Write a haiku.", GenerateTextOptions::default()).await?;
let mut stream = result.stream;
while let Some(part) = stream.next().await {
    match part? {
        StreamPart::TextDelta { delta, .. } => print!("{}", delta),
        StreamPart::Finish { .. } => println!("\n[done]"),
        _ => {}
    }
}
```

Streaming honors the same `timeout` / `abort_signal` options as
[text generation](#text-generation); streamed timeouts surface as
`StreamPart::Error { error: AiMuxError::Timeout(..) }` and aborting the
signal ends the stream with `StreamPart::Error { error: AiMuxError::Aborted }`.

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
use aimux_core::embedding_model::{EmbeddingCallOptions, EmbeddingModel};

let model = provider.embedding_model("text-embedding-3-small");
let opts = EmbeddingCallOptions::new("hello");
let result = model.do_embed(&opts).await?;
// result.embeddings[0] is Vec<f32>
```

## Speech Synthesis (TTS)

Converts text into speech audio.

```rust
use aimux_core::speech_model::{SpeechCallOptions, SpeechModel};

let model = provider.speech("tts-1");
let opts = SpeechCallOptions::new("Hello world!");
let result = model.do_generate(&opts).await?;
// result.audio is AudioData::Base64(String) or AudioData::Binary(Vec<u8>)
```

## Speech to Text (STT)

Converts audio into text (non-streaming).

```rust
use aimux_core::transcription_model::{AudioInput, TranscriptionCallOptions, TranscriptionModel};

let model = provider.transcription("whisper-1");
let opts = TranscriptionCallOptions::new(
    AudioInput::Base64(audio_base64),
    "audio/mp3",
);
let result = model.do_generate(&opts).await?;
// result.text, result.segments, result.language
```

## Image Generation

```rust
use aimux_core::image_model::{ImageCallOptions, ImageModel};

let model = provider.image("dall-e-3");
let opts = ImageCallOptions { prompt: Some("A cute sea otter".into()), n: 1, .. };
let result = model.do_generate(&opts).await?;
// result.images is ImageOutputs::Base64(Vec<String>) or Binary(Vec<Vec<u8>>)
```

## Video Generation

Video generation typically returns a URL (not binary).

```rust
use aimux_core::video_model::{VideoCallOptions, VideoModel};

let model = provider.video("veo-3.0");
let opts = VideoCallOptions { prompt: Some("A cat".into()), n: 1, .. };
let result = model.do_generate(&opts).await?;
// result.videos[0] is VideoData::Url { url, media_type }
```

## Reranking

Reorders a document list by relevance.

```rust
use aimux_core::reranking_model::{RerankingCallOptions, RerankingDocuments, RerankingModel};

let model = provider.reranking_model("rerank-v3.0");
let opts = RerankingCallOptions::new("What is Rust?", docs);
let result = model.do_rerank(&opts).await?;
// result.ranking sorted by score
```

## Search

Calls a search provider to obtain results.

```rust
use aimux_core::search_model::{SearchCallOptions, SearchModel};

let model = provider.search_model("tavily-search");
let opts = SearchCallOptions::new("What is Rust?");
let result = model.do_search(&opts).await?;
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

The Rust core provides 10 traits/interfaces, implemented by each provider as needed:

| Trait | Method | Semantics |
|-------|------|------|
| `Provider` | `name`, `language_model` | Provider factory — holds API config, creates `LanguageModel` instances by model name |
| `LanguageModel` | `do_generate`, `do_stream` | Text generation |
| `EmbeddingModel` | `do_embed` | Vector embedding |
| `SpeechModel` | `do_generate` | Speech synthesis |
| `TranscriptionModel` | `do_generate`, `do_stream` | Speech to text |
| `ImageModel` | `do_generate` | Image generation |
| `RerankingModel` | `do_rerank` | Reranking |
| `VideoModel` | `do_generate` | Video generation |
| `SearchModel` | `do_search` | Search |
| `Files` | `upload_file` | File upload |

The user-facing API consists of the `generate_text()` / `stream_text()` free functions, which internally call the trait methods.

> The multimodal accessors in the examples (`provider.embedding_model(...)`,
> `provider.speech(...)`, `provider.image(...)`, `provider.transcription(...)`,
> `provider.files()`, `provider.reranking_model(...)`,
> `provider.search_model(...)`, `provider.video(...)`) are **inherent methods
> on each provider struct**, not trait methods — they exist only on providers
> that support the feature (e.g. `OpenAIProvider` has `embedding_model` /
> `speech` / `image` / `transcription` / `files`; `CohereProvider` has
> `embedding_model` / `reranking_model`; `BedrockProvider` has `image`).

## Types

Rust types are the canonical definitions — one module per feature in
`aimux-core/src/`, re-exported through `aimux_core::prelude`:

| Module | Key types |
|------|------|
| `generate` | `generate_text` / `stream_text` functions, `GenerateResult` |
| `language_model` | `LanguageModel`, `GenerateTextResult` |
| `stream_part` | `StreamPart` (18 variants) |
| `options` | `GenerateTextOptions`, `CallOptions`, `ResponseFormat`, `ToolChoice`, `ReasoningEffort` |
| `message` / `language_model_message` | `ModelMessage`, `ModelPrompt`, `MessageContent`, `Role` |
| `content` | `ContentPart` (Text / Image / File / Reasoning / ToolCall / ToolResult) |
| `tool` | `Tool`, `FunctionTool`, `ProviderTool`, `ToolCall`, `ToolResult` |
| `types` | `Usage`, `TokenUsage`, `FinishReason`, `ResponseMetadata`, `Warning` |
| `shared` | `FileBytes`, `FileData`, `Size`, `AspectRatio`, `ResponseInfo`, `AbortSignal` |
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
