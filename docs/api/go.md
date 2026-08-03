# aimux · Go API

> Unified LLM service access layer — one API to access 325 AI providers

The Go binding goes through the `aimux-ffi` C ABI: cgo statically links
`libaimux_ffi.a`, so the Rust core is compiled into the executable and the
result is a single binary. See [RFC-0011](../../rfc/0011-golang-bindings.md)
for the design.

Shared reference — parameter tables, result shapes, factory functions, and the
feature coverage matrix — lives in the [API overview](../API.md).

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
```

## Providers

All 250 registry-backed OpenAI-compatible providers are reachable by name;
`aimux.ProviderName` holds typed constants:

> **Scope:** `provider(name)` covers only the 250 registry OpenAI-compatible
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
for part := range stream.Parts() {
    fmt.Println(part) // StreamPart JSON
}
```

> Stream part variants are documented in the [API overview](../API.md#streaming-generation).

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
method that drops the underlying FFI handle.

### Constructors

| Constructor | Returns | Notes |
|------|------|------|
| `NewOpenAI` / `NewOpenAIWithBase` | `*Model` | plus `OpenAI` / `OpenAIWithBase` (unchecked) |
| `NewAnthropic` / `NewAnthropicWithBase` | `*Model` | plus `Anthropic` / `AnthropicWithBase` |
| `NewDeepSeek` / `DeepSeek` | `*Model` | DeepSeek uses its official base URL |
| `NewOpenAIEmbedding(key, modelID)` | `*EmbeddingModel` | OpenAI only (Cohere/Google embedding have no Go constructor yet — use the C ABI) |
| `NewOpenAISpeech(key, modelID)` | `*SpeechModel` | |
| `NewOpenAITranscription(key, modelID)` | `*TranscriptionModel` | |
| `NewOpenAIImage(key, modelID)` | `*ImageModel` | OpenAI only (Google image has no Go constructor yet — use the C ABI) |
| `NewGoogleVideo(key, modelID)` | `*VideoModel` | |
| `NewCohereReranking(key, modelID)` | `*RerankingModel` | |
| `NewTavilySearch(key)` | `*SearchModel` | no model ID needed |
| `NewOpenAIFiles(key)` | `*Files` | |

### Methods

| Type | Methods | Boundary |
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

The multimodal methods return JSON strings (the FFI boundary); the
`ParseXxxResult` functions decode them into the typed structs. All call-option
pointer fields (`*string`, `*bool`, `*int`) are optional — pass `nil` to omit.
