# aimux · Go API

> Unified LLM service access layer — one API to access 325 AI providers

The Go binding goes through the `aimux-ffi` C ABI: cgo statically links
`libaimux_ffi.a`, so the Rust core is compiled into the executable and the
result is a single binary. See [RFC-0011](../../rfc/0011-golang-bindings.md)
for the design.

Shared reference — parameter tables, result shapes, factory functions, and the
feature coverage matrix — lives in the [API overview](../API.md).

## Install

```bash
go get github.com/arcships/aimux/bindings/go
go generate github.com/arcships/aimux/bindings/go   # downloads libaimux_ffi.a for your platform
go build ./...
```

Details (version pinning, unsupported platforms):
[bindings/go/README.md](../../bindings/go/README.md).

Requires **Go 1.23+**.

## Errors

Go follows openai-go / anthropic-sdk-go: **one `*aimux.Error` struct**
implementing `error` (not a class tree). Inspect with `errors.As`:

```go
result, err := model.GenerateText(`"hi"`, "")
if err != nil {
    var e *aimux.Error
    if errors.As(err, &e) {
        // e.Code, e.Message, e.Status, e.RetryMs
        if e.Code == aimux.CodeAPICall { // every HTTP-shaped failure
            switch e.Status {
            case 429:
                // rate limited; e.RetryMs may be a delay hint
            case 401:
                // auth failure
            case 404:
                // model not found
            }
        }
    }
    return err
}
```

| Field | Meaning |
|-------|---------|
| `Code` | error code (`CodeAPICall`, `CodeTokenExpired`, …; matches C `aimux_error_code_t`) |
| `Message` | human-readable text |
| `Status` | HTTP status, or `-1` |
| `RetryMs` | rate-limit hint, or `-1` (`0` = retry now) |
| `Retryable` | the core's retry verdict; never derived from `Status` |
| `ProviderCode`, `ProviderMessage`, `RequestID`, `ResponseBody` | `CodeAPICall` payload; empty under any other code |
| `ModelID`, `ModelType` | `CodeNoSuchModel` payload; empty under any other code |
| `ProviderID` | `CodeNoSuchProvider` payload; empty under any other code |

`Code` values 1..13 mirror aimux-core's `AiMuxError` variants. A code outside
the enum is a header/library mismatch and fails with a `panic`, not an error
type.

Recording failures are a separate type, as in Rust (`recording::RecordingError`
is unrelated to `AiMuxError`): `RecordingTryFlush() error` returns
`*aimux.RecordingError{Code RecordingErrorCode, Message string}`, with `Code`
one of `RecordingErrorInit … RecordingErrorWrite` (matches C
`aimux_error_code_t`; only `WriterGone`, `FlushTimeout`, `Write` are
reachable from a flush). `InitRecording(dir) error` reports construction
failures (`Init`, `OpenFile`, `Spawn`) as the same `*aimux.RecordingError`; on
failure the previous recorder stays in place. Inspect with
`errors.As(err, &re)`. It is not an `*aimux.Error`; its `Code` belongs to
`RecordingErrorCode`, not the core `aimux.Code` enum.
The legacy `RecordingFlush()` stays and never reports.

The two types — `*Error`, `*RecordingError` — share no base beyond `error`;
`errors.As` for one never matches another. There is no third aimux type.

**C ABI failures** ("your call, not the model") have codes 200–206 in C but
no Go type of their own; the binding maps them to native Go errors:

| Failure | Go |
|---------|----|
| bad raw JSON in `promptJson` / `optsJson` / `configJSON` | plain `error` naming the parameter, e.g. `aimux: prompt_json: invalid JSON` — checked in Go before the C call with `json.Valid` **plus** a surrogate-pairing scan, because `json.Valid` accepts unpaired `\uD800`–`\uDFFF` escapes that serde_json rejects (required parameters reject `""`; optional `""` = default; JSONL by line) |
| a string parameter that is not valid UTF-8, or contains a NUL | plain `error`: `aimux: <param>: must be valid UTF-8` / `aimux: <param>: must not contain NUL` — a Go string is an arbitrary byte sequence and `C.CString` passes it through verbatim (a NUL would truncate the argument silently), so every user-supplied string is checked before the C call. Two exceptions, both deliberate: the five `mustNew` constructors panic instead (see below), and `InitLogging` has no error channel and falls back to `"warn"` |
| a typed option struct (`ProviderConfig`, `EmbeddingCallOptions`, `SpeechCallOptions`, … , `TranscriptionSessionOpts`) whose raw JSON field carries bad bytes | plain `error` naming the marshalled parameter, e.g. `aimux: opts: invalid JSON: unpaired high surrogate \uD800`, `aimux: config_json: must be valid UTF-8` — `json.Marshal` is **not** self-validating: it coerces invalid UTF-8 in a Go *string* field to U+FFFD and escapes NUL, but a `json.RawMessage` field (`ProviderOptions`, `BodyOverrides`, `Audio`, `Documents`, `Files`, `Mask`, `Data`, `InputSchema`) is emitted through `compact()`, which checks JSON *syntax only*. Raw non-UTF-8 bytes and lone `\uD800`–`\uDFFF` escapes therefore survive marshalling, so every marshalled C argument gets the same `checkJSON` as a raw-string parameter |
| use-after-close (`Model`, `ProviderHandle`, multimodal models, `TranscriptionSession`) | `errors.Is(err, aimux.ErrClosed)` — guarded in Go before the C call |
| a `nil` `*Model` handed to a composite constructor — a `NewRouter` child, a `NewMoa` reference, or the `NewMoa` aggregator | plain `error` naming the position: `aimux: router: models[2] is nil`, `aimux: moa: references[1] is nil`, `aimux: moa: aggregator is nil` — checked in Go before the C call, so a nil element is a returned error rather than the nil-pointer dereference that would otherwise take the process down |
| trace query (`TraceAggregate`, `TraceSessionChain`, `TraceExportJsonl`, `TraceClear`) on a model that never went through `Trace` / `TraceAudited` | `errors.Is(err, aimux.ErrNotTraced)` — guarded in Go before the C call, ahead of argument validation; the trace store is keyed on the wrapper handle, so C can only report it as a missing handle |
| C code 200–206 (NULL / non-UTF-8 argument, malformed wire JSON, dead handle, re-entrant call, result serialization, callback failure) | returned as a plain `error` (not `*Error`, not `*RecordingError`, not `ErrClosed`): `aimux: <the C message>` |

Decoder: every fallible C call returns an opaque `aimux_error_t *` (NULL =
success, result in the out-parameter). One `aimux_error_code()` distinguishes
`AiMuxError` (1–13), `RecordingError` (100–105), and C ABI failures (200–206).
`expectAimuxError`, `expectRecordingError`, and `expectFfiError` enforce the
range expected by each call; the first two restore `*Error` and
`*RecordingError`, while 200–206 becomes a plain `error`. Every path frees the
getter strings and calls
`aimux_error_free` exactly once; errors are never handles. Nothing of
this leaks into the Go API.

The binding has five panic sites. Four are unreachable from Go input; the
fifth is the documented `Must`-style API, which panics on invalid input **by
design** — that one is opt-in, and every one of its five entry points has a
`NewXxx` twin that returns the same failure as an `error` instead.

| Panic site | Reachable from Go input? |
|------------|--------------------------|
| `aimux.go` `mustNew` — behind `OpenAI` / `OpenAIWithBase` / `Anthropic` / `AnthropicWithBase` / `DeepSeek` | **Yes, by design.** `regexp.MustCompile` convention: an `apiKey` / `modelID` / `baseURL` that is not valid UTF-8 or contains a NUL panics, as does any AiMuxError failure. Use `NewOpenAI` / `NewOpenAIWithBase` / `NewAnthropic` / `NewAnthropicWithBase` / `NewDeepSeek` for anything caller-supplied |
| `aimux.go` `InitLogging` — `expectFfiError` returned an error | No. `level` is coerced first: empty, non-UTF-8, or NUL-bearing falls back to `"warn"`, which is what aimux-core does with an unparseable level anyway (`AIMUX_LOG` / `AIMUX_LOG_LEVEL` outrank it regardless). That leaves no documented failure for `aimux_init_logging`, so a non-nil error here is a header/library mismatch |
| `aimux.go` `expectAimuxError` — `aimux_error_code_t` outside 1..13 | No. Header/library version mismatch |
| `aimux.go` `expectRecordingError` — `aimux_error_code_t` outside the enum | No. Header/library version mismatch |
| `multimodal.go` `TranscriptionSession.NextPart` — unknown `aimux_transcription_next_part` state | No. Header/library version mismatch |

The three mismatch panics are a contract violation the C header itself says to
abort on, not an error to report.

## Quick Start

```go
// cgo statically links libaimux_ffi.a, producing a single binary (the Rust core is compiled into the executable)
model := aimux.OpenAIWithBase("sk-...", "gpt-4o", "http://localhost:3000")
defer model.Close()
result, err := model.GenerateText(`"What is Rust?"`, "")
if err != nil {
    log.Fatal(err)
}
fmt.Println(result)
// streaming (typed: model.Stream(prompt, opts) yields *StreamPart values)
stream := model.StreamText(`"Write a haiku"`, "")
for part := range stream.Parts() {
    fmt.Println(part) // StreamPart JSON
}
if err := stream.Err(); err != nil { // only after Parts has closed
    log.Fatal(err)
}
```

Drain `Parts()` completely, or call `Cancel()` when stopping early. Context
variants connect cancellation automatically. `Cancel` releases callback
backpressure and aborts the native request; it never closes `Parts()` itself.
The producer closes `Parts()` after the blocking native call returns.

## Providers

All 251 registry-backed OpenAI-compatible providers are reachable by name;
`aimux.ProviderName` holds typed constants:

> **Scope:** `provider(name)` covers only the 251 registry OpenAI-compatible
> providers; Anthropic/Google/multimodal/local → typed constructors
> (`NewAnthropic(apiKey, model)`); custom endpoints → `WithBase` variant.
> Full list: [providers.md](providers.md).

```go
// 推荐:aimux.Groq 类型常量(类型检查 + 补全)
model, err := aimux.Provider(string(aimux.Groq), "", "llama-3.3-70b")
if err != nil { log.Fatal(err) }
defer model.Close()

// 字符串形式同样可用 + base URL 覆盖:
model2, err := aimux.ProviderWithBase("groq", "sk-...", "llama-3.3-70b", "https://relay.example/v1")
defer model2.Close()

// 完整 ProviderOptions(base_url / headers / organization / project /
// max_retries / body_overrides):
model3, err := aimux.ProviderWithConfig("groq", "sk-...", "llama-3.3-70b", &aimux.ProviderConfig{
	Headers: map[string]string{"X-Custom": "1"},
})
defer model3.Close()
```

`NewDeepSeek` / `DeepSeek` remain as shortcuts (registry-backed). Unknown
names return an error.

## Text Generation

Non-streaming text generation; returns the complete result.

```go
model := aimux.OpenAIWithBase("sk-...", "gpt-4o", "http://localhost:3000")
defer model.Close()
result, err := model.GenerateText(`"What is Rust?"`, "")
if err != nil {
    log.Fatal(err)
}
fmt.Println(result)
```

> Parameters, return value, and the `raw.content` variants are documented in
> the [API overview](../API.md#text-generation).

## Streaming Generation

Returns generated content as a stream, output chunk by chunk.

```go
stream := model.StreamText(`"Write a haiku"`, "")
defer stream.Cancel()

for part := range stream.Parts() {
    fmt.Println(part) // StreamPart JSON
}
if err := stream.Err(); err != nil {
    log.Fatal(err)
}
```

> Stream part variants are documented in the [API overview](../API.md#streaming-generation).
> Drain `Parts()` before calling `Err()`. If you stop reading early, call
> `Cancel()` so the native stream does not keep running.

## Vector Embedding

Converts text into a vector representation.

```go
// Typed API — Embed() takes []string, returns a JSON string you can parse
// with ParseEmbeddingResult.
embedder, err := aimux.NewOpenAIEmbedding("sk-...", "text-embedding-3-small")
if err != nil {
    log.Fatal(err)
}
defer embedder.Close()

resultJSON, err := embedder.Embed([]string{"hello", "world"}, nil)
if err != nil {
    log.Fatal(err)
}
result, err := aimux.ParseEmbeddingResult(resultJSON)
if err != nil {
    log.Fatal(err)
}
fmt.Println(len(result.Embeddings))       // 2
fmt.Println(len(result.Embeddings[0]))    // 1536 (dimension depends on model)
```

## Speech Synthesis (TTS)

Converts text into speech audio.

```go
voice := "alloy"
outputFormat := "mp3"

speaker, err := aimux.NewOpenAISpeech("sk-...", "tts-1")
if err != nil {
    log.Fatal(err)
}
defer speaker.Close()

resultJSON, err := speaker.Generate(&aimux.SpeechCallOptions{
    Text:         "Hello world!",
    Voice:        &voice,    // optional *string fields — pass a pointer or nil
    OutputFormat: &outputFormat,
})
if err != nil {
    log.Fatal(err)
}
result, err := aimux.ParseSpeechResult(resultJSON)
if err != nil {
    log.Fatal(err)
}
// audio bytes: *result.Audio.Base64 (base64 string) or result.Audio.Binary
```

## Speech to Text (STT)

Converts audio into text (non-streaming).

```go
transcriber, err := aimux.NewOpenAITranscription("sk-...", "whisper-1")
if err != nil {
    log.Fatal(err)
}
defer transcriber.Close()

// audioBase64 is base64-encoded audio; media type e.g. "audio/mp3"
resultJSON, err := transcriber.Generate(audioBase64, "audio/mp3", nil)
if err != nil {
    log.Fatal(err)
}
result, err := aimux.ParseTranscriptionResult(resultJSON)
if err != nil {
    log.Fatal(err)
}
fmt.Println(result.Text)              // transcribed text
fmt.Println(result.Segments)          // timestamped segments
fmt.Println(*result.Language)         // detected language
```

## Image Generation

```go
prompt := "A cute baby sea otter"
n := 1

imager, err := aimux.NewOpenAIImage("sk-...", "dall-e-3")
if err != nil {
    log.Fatal(err)
}
defer imager.Close()

resultJSON, err := imager.Generate(&aimux.ImageCallOptions{
    Prompt: &prompt,
    N:      &n,
})
if err != nil {
    log.Fatal(err)
}
result, err := aimux.ParseImageResult(resultJSON)
if err != nil {
    log.Fatal(err)
}
// result.Images.Base64[0] (base64) or result.Images.Binary[0] (raw bytes)
```

## Video Generation

Video generation typically returns a URL (not binary).

```go
prompt := "A cat playing piano"
n := 1

videor, err := aimux.NewGoogleVideo("sk-...", "veo-3.0")
if err != nil {
    log.Fatal(err)
}
defer videor.Close()

resultJSON, err := videor.Generate(&aimux.VideoCallOptions{
    Prompt: &prompt,
    N:      &n,
})
if err != nil {
    log.Fatal(err)
}
result, err := aimux.ParseVideoResult(resultJSON)
if err != nil {
    log.Fatal(err)
}
// result.Videos[0].Url.URL — video URL
```

## Reranking

Reorders a document list by relevance.

```go
topN := 3

reranker, err := aimux.NewCohereReranking("sk-...", "rerank-v3.0")
if err != nil {
    log.Fatal(err)
}
defer reranker.Close()

resultJSON, err := reranker.Rerank(&aimux.RerankingCallOptions{
    Query:     "What is Rust?",
    Documents: json.RawMessage(`[{"text":"Rust is a systems programming language."},{"text":"Rust is a chemical element."}]`),
    TopN:      &topN,
})
if err != nil {
    log.Fatal(err)
}
result, err := aimux.ParseRerankingResult(resultJSON)
if err != nil {
    log.Fatal(err)
}
// result.Ranking sorted by relevance score
for _, rank := range result.Ranking {
    fmt.Println(rank.Index, rank.RelevanceScore)
}
```

## Search

Calls a search provider to obtain results.

```go
maxResults := 5

searcher, err := aimux.NewTavilySearch("tvly-...")
if err != nil {
    log.Fatal(err)
}
defer searcher.Close()

resultJSON, err := searcher.Search(&aimux.SearchCallOptions{
    Query:      "What is Rust?",
    MaxResults: &maxResults,
})
if err != nil {
    log.Fatal(err)
}
result, err := aimux.ParseSearchResult(resultJSON)
if err != nil {
    log.Fatal(err)
}
// result.Results is []SearchResultItem; result.Answer is *string (may be nil)
for _, item := range result.Results {
    fmt.Println(*item.Title, *item.URL)
}
```

## File Upload

Uploads a file to the provider and returns a file ID.

```go
files, err := aimux.NewOpenAIFiles("sk-...")
if err != nil {
    log.Fatal(err)
}
defer files.Close()

// dataBase64 is base64-encoded file content; media type e.g. "application/pdf"
resultJSON, err := files.Upload(dataBase64, "application/pdf", nil)
if err != nil {
    log.Fatal(err)
}
result, err := aimux.ParseUploadFileResult(resultJSON)
if err != nil {
    log.Fatal(err)
}
fmt.Println(result.ProviderReference)  // map["openai":"file-xxx"]
```

## API Surface

All constructors come in two flavors: `NewXxx(...) (T, error)` (checked) and
`Xxx(...) T` (unchecked, panics on failure). Every model type has a `Close()`
method that atomically drops the underlying FFI handle. Handle wrappers must
not be copied after first use. `Close` is idempotent and does not wait for an
in-flight network call or stream; a racing call either enters Rust first and
continues with its cloned `Arc`, or receives an invalid-handle error.

### Constructors

| Constructor | Returns | Notes |
|------|------|------|
| `NewOpenAI` / `NewOpenAIWithBase` | `*Model` | plus `OpenAI` / `OpenAIWithBase`, which **panic** on any failure, invalid input included |
| `NewAnthropic` / `NewAnthropicWithBase` | `*Model` | plus `Anthropic` / `AnthropicWithBase`, which **panic** on any failure, invalid input included |
| `NewDeepSeek` | `*Model` | DeepSeek uses its official base URL; `DeepSeek` is the **panicking** twin |
| `NewOpenAIEmbedding` / `NewCohereEmbedding` / `NewGoogleEmbedding` `(key, modelID)` | `*EmbeddingModel` | each has a `…WithBase` variant |
| `NewOpenAISpeech(key, modelID)` | `*SpeechModel` | |
| `NewOpenAITranscription(key, modelID)` | `*TranscriptionModel` | |
| `NewOpenAIImage` / `NewGoogleImage` `(key, modelID)` | `*ImageModel` | each has a `…WithBase` variant |
| `NewGoogleVideo(key, modelID)` | `*VideoModel` | |
| `NewCohereReranking(key, modelID)` | `*RerankingModel` | |
| `NewTavilySearch(key)` | `*SearchModel` | no model ID needed |
| `NewOpenAIFiles(key)` | `*Files` | |
| `NewRouter(models []*Model, configJSON)` | `*Model` | RFC-0021 fallback router over `models` (must be non-empty); the same model may appear more than once |
| `NewMoa(references []*Model, aggregator *Model, configJSON)` | `*Model` | RFC-0022 mixture-of-agents; `references` may be empty, may repeat a model, and may contain the aggregator |

#### Composite constructors and concurrency

`NewRouter` and `NewMoa` snapshot each atomic handle in caller order and never
hold a Go lifecycle lock while calling C. Duplicate models require no special
case, and opposite caller orders cannot form an ABBA cycle because there is no
multi-lock protocol. Concurrent `Close` is resolved by the Rust registry:
construction either clones a model `Arc` or returns an invalid-handle error.

- **A `nil` element is a returned error**, checked before the C call:
  `aimux: router: models[i] is nil`, `aimux: moa: references[i] is nil`,
  `aimux: moa: aggregator is nil`.

Both constructors take a new reference to each child, so the caller keeps
ownership: closing a child afterwards does not invalidate the composite, and
the composite must be closed separately.

### Methods

| Type | Methods | Result |
|------|------|------|
| `*Model` | `GenerateText(promptJson, optsJson) (string, error)` — raw JSON; `Generate(prompt any, opts *GenerateTextOptions) (*GenerateTextResult, error)` — typed | typed input via `Generate` |
| `*Model` | `StreamText(promptJson, optsJson) *Stream` — raw JSON parts; `Stream(prompt any, opts *GenerateTextOptions) (*TypedStream, error)` — typed `*StreamPart` values | `Stream.Parts()` / `TypedStream.Parts()` channels, `.Err()` |
| `*EmbeddingModel` | `Embed(values []string, opts *EmbeddingCallOptions) (string, error)` | returns JSON; use `ParseEmbeddingResult` |
| `*SpeechModel` | `Generate(opts *SpeechCallOptions) (string, error)` | `ParseSpeechResult` |
| `*ImageModel` | `Generate(opts *ImageCallOptions) (string, error)` | `ParseImageResult` |
| `*TranscriptionModel` | `Generate(audioBase64, mediaType string, opts *TranscriptionCallOptions) (string, error)` | `ParseTranscriptionResult` |
| `*VideoModel` | `Generate(opts *VideoCallOptions) (string, error)` | `ParseVideoResult` |
| `*RerankingModel` | `Rerank(opts *RerankingCallOptions) (string, error)` | `ParseRerankingResult` |
| `*SearchModel` | `Search(opts *SearchCallOptions) (string, error)` | `ParseSearchResult` |
| `*Files` | `Upload(dataBase64, mediaType string, opts *UploadFileCallOptions) (string, error)` | `ParseUploadFileResult` |

`opts` is required (non-nil) for `SpeechModel.Generate`, `ImageModel.Generate`, `VideoModel.Generate`, `Rerank` and `Search` — it carries the input; `nil` returns `aimux: <Method>: opts is required`. `Embed`, `TranscriptionModel.Generate` and `Upload` accept `nil` opts (defaults). `InitProxy` requires non-empty JSON (`"{}"` for defaults); `TraceAggregate("")` means "all". The trace queries (`TraceAggregate`, `TraceSessionChain`, `TraceExportJsonl`, `TraceClear`) run only on the `*Model` returned by `Trace()` / `TraceAudited()`; on any other model they return `aimux.ErrNotTraced`.

## Types

Typed structs live in `bindings/go/types.go` (text) and
`bindings/go/multimodal_types.go` (multimodal): `GenerateTextOptions`,
`GenerateTextResult`, `StreamPart`, `ModelMessage`, `Tool`, `ToolChoice`
(with helpers `ToolChoiceAuto()` / `ToolChoiceNone()`), `ToolCall`,
`ToolResult`, `Usage`, `FinishReason`, `Role`, `MessageContent`, `ContentPart`,
`ResponseFormat`, `ReasoningEffort`, `Warning`, `GenerateResult`, plus
`EmbeddingCallOptions/Result`, `SpeechCallOptions/Result`,
`ImageCallOptions/Result`, `TranscriptionCallOptions/Result`,
`VideoCallOptions/Result`, `RerankingCallOptions/Result`,
`SearchCallOptions/Result`, `UploadFileCallOptions/Result`.

The multimodal methods return JSON strings through the C ABI; the
`ParseXxxResult` functions decode them into the typed structs. All call-option
pointer fields (`*string`, `*bool`, `*int`) are optional — pass `nil` to omit.
