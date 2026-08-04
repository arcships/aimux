# aimux · C ABI (C/C++)

> The C ABI boundary (`aimux-ffi`) provides FFI interfaces for Swift / Kotlin / Flutter / Go / C++. All functions communicate via JSON strings.

Shared reference — feature descriptions, factory functions, and the coverage
matrix — lives in the [API overview](../API.md).

## Install

Get `aimux-ffi.h` + the platform shared library from
[GitHub Releases](https://github.com/arcships/aimux/releases)
(`libaimux_ffi-linux-x64.so` / `libaimux_ffi-macos-arm64.dylib` /
`aimux_ffi-windows-x64.dll`), then link against it:

```bash
gcc -o example example.c -I. -L. -laimux_ffi -lpthread -ldl -lm   # Linux (aimux-ffi.h in the same directory)
```

Header: [aimux-ffi.h](../../aimux-ffi/aimux-ffi.h).

## Quick Start

```c
#include "aimux-ffi.h"

uint64_t handle = aimux_openai_new("sk-...", "gpt-4o");
// prompt_json is a JSON string; opts_json is a GenerateTextOptions JSON
const char *result = aimux_generate_text(handle, "\"What is Rust?\"", "{}");
// result is a GenerateResult JSON string
aimux_free_string(result);
aimux_drop_handle(handle);
```

## Function List

### Language Model

| Function | Description |
|------|------|
| `aimux_openai_new(api_key, model_id)` | Create an OpenAI language model |
| `aimux_openai_new_with_base(api_key, model_id, base_url)` | Create an OpenAI language model (custom base_url, for mock testing) |
| `aimux_anthropic_new(api_key, model_id)` | Create an Anthropic language model |
| `aimux_anthropic_new_with_base(api_key, model_id, base_url)` | Create an Anthropic language model (custom base_url) |
| `aimux_cohere_new(api_key, model_id)` / `aimux_cohere_new_with_base(...)` | Create a Cohere language model |
| `aimux_mistral_new(api_key, model_id)` / `aimux_mistral_new_with_base(...)` | Create a Mistral language model |
| `aimux_xai_new(api_key, model_id)` / `aimux_xai_new_with_base(...)` | Create an xAI language model |
| `aimux_bedrock_new(access_key_id, secret_access_key, region, model_id)` / `aimux_bedrock_new_with_base(..., base_url)` | Create a Bedrock language model (AWS SigV4 credentials) |
| `aimux_vertex_new(access_token, project, location, model_id)` / `aimux_vertex_new_with_base(..., base_url)` | Create a Vertex AI language model (GCP bearer token) |
| `aimux_anthropic_aws_new(api_key, region, model_id)` / `aimux_anthropic_aws_new_with_base(..., base_url)` | Create an Anthropic-on-AWS language model |
| `aimux_azure_new(api_key, resource_name, deployment, api_version)` / `aimux_azure_new_with_base(api_key, base_url, deployment, api_version)` | Create an Azure OpenAI language model (`api_version` NULL = default) |
| `aimux_provider_new(name, api_key, model_id, config_json)` | Create a language model from the built-in registry by provider name (`api_key` may be NULL to read the provider's env var; `config_json` optional `{"base_url":...}` JSON or NULL) |
| `aimux_provider_from_env(name, model_id)` | Create a registry language model, reading the API key from the provider's env var |
| `aimux_generate_text(handle, prompt_json, opts_json)` | Non-streaming generation (returns a JSON string) |
| `aimux_stream_text(handle, prompt_json, opts_json, on_part, on_done, on_error)` | Streaming generation (push callback) |

### Vector Embedding

| Function | Description |
|------|------|
| `aimux_openai_embedding_new(api_key, model_id)` | Create an OpenAI embedding model |
| `aimux_openai_embedding_new_with_base(api_key, model_id, base_url)` | Create an OpenAI embedding model (custom base_url) |
| `aimux_cohere_embedding_new(api_key, model_id)` | Create a Cohere embedding model |
| `aimux_cohere_embedding_new_with_base(api_key, model_id, base_url)` | Create a Cohere embedding model (custom base_url) |
| `aimux_google_embedding_new(api_key, model_id)` | Create a Google embedding model |
| `aimux_google_embedding_new_with_base(api_key, model_id, base_url)` | Create a Google embedding model (custom base_url) |
| `aimux_embed(handle, values_json, opts_json)` | Generate vector embeddings |

### Speech

| Function | Description |
|------|------|
| `aimux_openai_speech_new(api_key, model_id)` | Create a TTS model |
| `aimux_openai_speech_new_with_base(api_key, model_id, base_url)` | Create a TTS model (custom base_url) |
| `aimux_speech_generate(handle, opts_json)` | Generate speech |
| `aimux_openai_transcription_new(api_key, model_id)` | Create an STT model |
| `aimux_openai_transcription_new_with_base(api_key, model_id, base_url)` | Create an STT model (custom base_url) |
| `aimux_transcription_generate(handle, audio_base64, media_type, opts_json)` | Transcribe audio |

### Image

| Function | Description |
|------|------|
| `aimux_openai_image_new(api_key, model_id)` | Create an OpenAI image model |
| `aimux_openai_image_new_with_base(api_key, model_id, base_url)` | Create an OpenAI image model (custom base_url) |
| `aimux_google_image_new(api_key, model_id)` | Create a Google image model |
| `aimux_google_image_new_with_base(api_key, model_id, base_url)` | Create a Google image model (custom base_url) |
| `aimux_image_generate(handle, opts_json)` | Generate an image |

### Video Generation (added 2026-07-29)

| Function | Description |
|------|------|
| `aimux_google_video_new(api_key, model_id)` | Create a Google video model |
| `aimux_google_video_new_with_base(api_key, model_id, base_url)` | Create a Google video model (custom base_url) |
| `aimux_video_generate(handle, opts_json)` | Generate a video (`VideoCallOptions` JSON) |

### Reranking (added 2026-07-29)

| Function | Description |
|------|------|
| `aimux_cohere_reranking_new(api_key, model_id)` | Create a Cohere reranking model |
| `aimux_cohere_reranking_new_with_base(api_key, model_id, base_url)` | Create a Cohere reranking model (custom base_url) |
| `aimux_rerank(handle, opts_json)` | Rerank (`RerankingCallOptions` JSON) |

### Search (added 2026-07-29)

| Function | Description |
|------|------|
| `aimux_tavily_search_new(api_key, model_id)` | Create a Tavily search model (`model_id` is a placeholder only; Tavily uses a fixed endpoint) |
| `aimux_tavily_search_new_with_base(api_key, model_id, base_url)` | Create a Tavily search model (custom base_url) |
| `aimux_search(handle, opts_json)` | Execute a search (`SearchCallOptions` JSON) |

### File

| Function | Description |
|------|------|
| `aimux_openai_files_new(api_key)` | Create a file manager |
| `aimux_openai_files_new_with_base(api_key, base_url)` | Create a file manager (custom base_url) |
| `aimux_file_upload(handle, data_base64, media_type, opts_json)` | Upload a file |

### Resource Management

| Function | Description |
|------|------|
| `aimux_drop_handle(handle)` | Free the model handle (0 is a no-op) |
| `aimux_free_string(ptr)` | Free a returned string |

## Examples

### Video Generation

```c
uint64_t handle = aimux_google_video_new(api_key, "veo-3.0");
// opts_json: {"prompt":"A cat playing piano","n":1}
const char *result = aimux_video_generate(handle, opts_json);
aimux_drop_handle(handle);
aimux_free_string(result);
```

### Reranking

```c
uint64_t handle = aimux_cohere_reranking_new(api_key, "rerank-v3.0");
// opts_json: {"query":"What is Rust?","documents":{"Text":{"values":["doc1","doc2"]}},"top_n":3}
const char *result = aimux_rerank(handle, opts_json);
aimux_drop_handle(handle);
aimux_free_string(result);
```

### Search

```c
uint64_t handle = aimux_tavily_search_new(api_key, "tavily-search");
// opts_json: {"query":"What is Rust?","max_results":5}
const char *result = aimux_search(handle, opts_json);
// result: {"results":[{"title":"...","url":"...","content":"..."}],"answer":null}
aimux_drop_handle(handle);
aimux_free_string(result);
```

## Memory Management

- `aimux_generate_text` and similar functions return `char*`; the caller must release it with `aimux_free_string`
- The `const char*` received by the `aimux_stream_text` callback is valid only during the callback; it must be copied synchronously within the callback
- `aimux_drop_handle` frees the model handle (0 is a no-op)

## Header File

`aimux-ffi/aimux-ffi.h` — the complete C header file; C++ can use it directly by wrapping it in `extern "C"`.
