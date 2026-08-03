# aimux · Python API

> Unified LLM service access layer — one API to access 325 AI providers

Shared reference — parameter tables, result shapes, factory functions, and the
feature coverage matrix — lives in the [API overview](../API.md).

## Quick Start

```bash
pip install aimux
```

```python
from aimux import openai, generate_text

model = openai("sk-...", "gpt-4o")
result = generate_text(model, "What is Rust?")
print(result["text"])
```

## Built-in Providers (RFC-0017 phase 4)

```python
from aimux import provider, generate_text

# Key from the provider's env var (GROQ_API_KEY etc.):
model = provider("groq", None, "llama-3.3-70b")
# Explicit key + base URL override:
model = provider("groq", "sk-...", "llama-3.3-70b", "https://relay.example/v1")
result = generate_text(model, "Hello")
```

`provider(name, api_key, model_id, base_url=None)` covers all 250 built-in
OpenAI-compatible providers. `openai` / `anthropic` / `deepseek` factories
remain (deepseek is now registry-backed).

> **Scope:** `provider(name)` covers only the 250 registry OpenAI-compatible
> providers; Anthropic/Google/multimodal/local → typed factories
> (`anthropic(api_key, model)`); custom endpoints → `base_url` param.
> Full list: [providers.md](providers.md).

## Text Generation

Non-streaming text generation; returns the complete result.

```python
from aimux import openai, generate_text

model = openai("sk-...", "gpt-4o")
result = generate_text(model, "Explain Rust ownership.", {
    "max_output_tokens": 100,
    "temperature": 0.7,
})

print(result["text"])
print(result["usage"])
print(result["finish_reason"])
```

> Parameters, return value, and the `raw.content` variants are documented in
> the [API overview](../API.md#text-generation).

## Streaming Generation

Returns generated content as a stream, output chunk by chunk.

```python
from aimux import openai, stream_text

model = openai("sk-...", "gpt-4o")
for part in stream_text(model, "Write a haiku about Rust."):
    if "TextDelta" in part:
        print(part["TextDelta"]["delta"], end="")
    if "Finish" in part:
        print("\n[done]")
```

> Stream part variants are documented in the [API overview](../API.md#streaming-generation).

## Tool Calling

Tool definitions are language-agnostic data descriptions (JSON Schema) that require no macros.

### Defining Tools

```python
# Python — the same data shape via the options dict
tools = [{
    "type": "function",
    "name": "get_weather",
    "description": "Get current weather",
    "input_schema": {
        "type": "object",
        "properties": {"location": {"type": "string", "description": "City name"}},
        "required": ["location"]
    }
}]

result = generate_text(model, "What's the weather in Tokyo?", {"tools": tools})
if len(result["tool_calls"]) > 0:
    call = result["tool_calls"][0]
    print(call["tool_name"])      # get_weather
    print(call["input"])          # {"location": "Tokyo"}
```

### Tool Selection Strategy

Pass `tool_choice` through the options dict:

```python
opts = {
    "tools": tools,
    "tool_choice": "auto"   # "auto" | "none" | "required" | {"type": "tool", "toolName": "get_weather"}
}
```

## Multi-Role Messages

`prompt` accepts a message array to implement multi-turn conversation; roles support `system` / `user` / `assistant` / `tool`:

```python
# Python — system + user multi-turn
result = generate_text(model, [
    {"role": "system", "content": "You are a helpful assistant."},
    {"role": "user", "content": "What is Rust?"},
])
```

## Vector Embedding

Converts text into a vector representation.

```python
from aimux import openai_embedding
import json

embedder = openai_embedding("sk-...", "text-embedding-3-small")
# embed() takes a JSON string, returns a JSON string
result = json.loads(embedder.embed(json.dumps(["hello", "world"])))
print(len(result["embeddings"]))      # 2
print(len(result["embeddings"][0]))   # 1536
```

## Speech Synthesis (TTS)

Converts text into speech audio.

```python
from aimux import openai_speech
import json, base64

speaker = openai_speech("sk-...", "tts-1")
result = json.loads(speaker.generate(json.dumps({
    "text": "Hello world!",
    "voice": "alloy",
    "output_format": "mp3",
})))

if "Base64" in result["audio"]:
    audio_bytes = base64.b64decode(result["audio"]["Base64"])
    with open("out.mp3", "wb") as f:
        f.write(audio_bytes)
```

## Speech to Text (STT)

Converts audio into text (non-streaming).

```python
from aimux import openai_transcription
import base64, json

transcriber = openai_transcription("sk-...", "whisper-1")
audio_b64 = base64.b64encode(open("audio.mp3", "rb").read()).decode()
result = json.loads(transcriber.generate(audio_b64, "audio/mp3"))

print(result["text"])
print(result["segments"])
```

## Image Generation

```python
from aimux import openai_image
import json, base64

imager = openai_image("sk-...", "dall-e-3")
result = json.loads(imager.generate(json.dumps({
    "prompt": "A cute baby sea otter",
    "n": 1,
})))

if "Base64" in result["images"]:
    with open("out.png", "wb") as f:
        f.write(base64.b64decode(result["images"]["Base64"][0]))
```

## Video Generation

Video generation typically returns a URL (not binary).

```python
from aimux import google_video
import json

videor = google_video("sk-...", "veo-3.0")
result = json.loads(videor.generate(json.dumps({
    "prompt": "A cat playing piano",
    "n": 1,
})))

# result["videos"] is usually [{"Url": {"url": "...", "media_type": "..."}}]
if "Url" in result["videos"][0]:
    print(result["videos"][0]["Url"]["url"])
```

## Reranking

Reorders a document list by relevance.

```python
from aimux import cohere_reranking
import json

reranker = cohere_reranking("sk-...", "rerank-v3.0")
result = json.loads(reranker.rerank(
    "What is Rust?",
    json.dumps([
        {"text": "Rust is a systems programming language."},
        {"text": "Rust is a chemical element."},
    ]),
    json.dumps({"top_n": 3}),
))

# result["ranking"] sorted by relevance_score
for rank in result["ranking"]:
    print(rank["index"], rank["relevance_score"])
```

## Search

```python
# Same as Node: the SearchModel class is exposed, but there is no factory
# function yet — use via the Rust core, the Go binding, or the C ABI
```

## File Upload

Uploads a file to the provider and returns a file ID.

```python
from aimux import openai_files
import base64, json

files = openai_files("sk-...")
file_b64 = base64.b64encode(open("doc.pdf", "rb").read()).decode()
result = json.loads(files.upload_file(file_b64, "application/pdf"))

print(result["provider_reference"])  # {"openai": "file-xxx"}
```

## API Surface

The `aimux` package has two layers:

| Layer | Source | Boundary |
|------|------|------|
| **Native (PyO3)** | `bindings/python/src/lib.rs` (`aimux.abi3.so`) | JSON strings in / JSON strings out |
| **Typed wrapper** | `bindings/python/python/aimux/wrapper.py` | pydantic models / Python dicts |

### Native classes and factory functions

| Class | Factory functions | Methods |
|------|------|------|
| `Model` | `openai` / `anthropic` / `deepseek` | `generate_text(prompt_json, opts_json=None)`, `stream_text(...)` |
| `EmbeddingModel` | `openai_embedding` / `cohere_embedding` / `google_embedding` | `embed(values_json, opts_json=None)` |
| `SpeechModel` | `openai_speech` | `generate(opts_json)` |
| `TranscriptionModel` | `openai_transcription` | `generate(audio_base64, media_type, opts_json=None)` |
| `ImageModel` | `openai_image` / `google_image` | `generate(opts_json)` |
| `VideoModel` | `google_video` | `generate(opts_json)` |
| `RerankingModel` | `cohere_reranking` | `rerank(query, docs_json, opts_json=None)` |
| `SearchModel` | — (no factory yet) | `search(query, opts_json=None)` |
| `Files` | `openai_files(api_key, base_url=None)` | `upload_file(data_base64, media_type, opts_json=None)` |
| `StreamIterator` | returned by `Model.stream_text` | `__iter__` / `__next__` of `StreamPart` JSON strings |

All factories accept an optional `base_url` and return instances synchronously
(no `await`). The typed wrapper adds three functions: `generate_text` (returns a
pydantic `GenerateTextResult`), `stream_text` (yields parsed `StreamPart`
dicts), and `parse_stream_part` (validates a dict into a typed `StreamPart`).

## Types

The wrapper's types are pydantic models in `bindings/python/python/aimux/wrapper.py`:

```python
from aimux.wrapper import (
    # type aliases
    Role, FinishReasonUnified, ReasoningEffort, MessageContent, ContentPart,
    Tool, ToolChoice, ResponseFormat, StreamPart, GenerateContent,
    FileData, FileBytes, Warning, AiMuxError,
    # pydantic models
    TokenUsage, Usage, FinishReason, ResponseMetadata, ToolCall,
    ModelMessage, FunctionTool, ProviderTool, TextContentPart,
    GenerateTextOptions, GenerateTextResult, GenerateResult,
    # functions
    generate_text, stream_text, parse_stream_part,
)
```

Errors from the native layer surface as `RuntimeError` (the wrapper does not
define its own exception type yet).

Key shapes (mirroring the shared JSON schema):

```python
class GenerateTextResult(BaseModel):
    text: str
    tool_calls: list[ToolCall]
    finish_reason: FinishReason
    usage: Usage
    warnings: list[Warning]
    raw: GenerateResult
```

`StreamPart` is a `RootModel` over the external-tagged union dict, e.g.
`{"TextDelta": {"id": ..., "delta": ...}}`. Iterate dicts with
`if "TextDelta" in part:` (as in [Streaming Generation](#streaming-generation))
or validate them with `parse_stream_part(part)` for attribute access.
