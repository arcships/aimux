# aimux · C ABI (C/C++)

> The `aimux-ffi` C ABI is shared by Swift / Kotlin / Flutter / Go / Java / C++.
> **Results** are JSON strings written to a trailing out-parameter. **Errors**
> are not JSON: every fallible function returns an opaque
> `aimux_error_t *` (`NULL` = success) with one code and message, released with
> `aimux_error_free` (see
> [aimux-error.h](../../aimux-ffi/aimux-error.h)).

Shared reference — feature descriptions, factory functions, and the coverage
matrix — lives in the [API overview](../API.md).

## Install

Get `aimux-ffi.h`, `aimux-error.h`, and the platform shared library from
[GitHub Releases](https://github.com/arcships/aimux/releases)
(`libaimux_ffi-linux-x64.so` / `libaimux_ffi-macos-arm64.dylib` /
`aimux_ffi-windows-x64.dll`), then link against it:

```bash
gcc -o example example.c -I. -L. -laimux_ffi -lpthread -ldl -lm
```

Headers:

- [aimux-ffi.h](../../aimux-ffi/aimux-ffi.h) — ABI symbols  
- [aimux-error.h](../../aimux-ffi/aimux-error.h) — the returned error, unified code enum, and getters

## Error handling

### How to tell failure

**Only the return value decides success or failure.**

| Return | Out-parameter | Meaning |
|--------|---------------|---------|
| `NULL` | written (non-zero handle / non-`NULL` string) | success |
| non-`NULL` `aimux_error_t *` | at its sentinel (`0` / `NULL`) | failure — you own the returned error |

```c
uint64_t h = 0;
aimux_error_t *e = aimux_openai_new(key, "gpt-4o", &h);
if (e) { /* failed; h == 0 */ }
```

Constructors write to `uint64_t *out_handle`; JSON results to `char **out_json`;
actions (`aimux_stream_text`, `aimux_init_logging`, `aimux_trace_clear`, …)
have no out-parameter. Out-parameters are written to their sentinel on entry;
a `NULL` out-parameter pointer is itself a C ABI failure. Functions that
cannot fail keep their natural signature (`void aimux_drop_handle`,
`uint64_t aimux_abort_signal_new(void)`).

Exception: `aimux_transcription_next_part` returns `NULL` for all three pull
states; `*out_part` is only non-`NULL` when `*out_state ==
AIMUX_TRANSCRIPTION_NEXT_PART_PART`. Check `*out_state` before reading
`*out_part`.

Do **not** sniff JSON for `"error"`.

### The returned error

```c
typedef struct aimux_error aimux_error_t;  /* opaque; owned */

void    aimux_error_free(aimux_error_t *err);          /* NULL-safe; exactly once */
int32_t aimux_error_code(const aimux_error_t *err);    /* AIMUX_OK for NULL */
char   *aimux_error_message(const aimux_error_t *err); /* owned → aimux_free_string */
```

The code range identifies the source:

| Range | Meaning |
|---|---|
| `1..13` | `AiMuxError` |
| `100..105` | `RecordingError` |
| `200..206` | failure detected by the C ABI |

`AIMUX_OK` (0) means the pointer is `NULL`; a non-`NULL` error never reports
0. Copy strings and fields before calling `aimux_error_free()`.

### Getters

```c
int32_t aimux_error_code(const aimux_error_t *err);        /* aimux_error_code_t; AIMUX_OK for NULL */
char   *aimux_error_message(const aimux_error_t *err);     /* every code; owned → aimux_free_string */
int32_t aimux_error_retryable(const aimux_error_t *err);   /* 1 = retrying may help; every code */
int32_t aimux_error_status(const aimux_error_t *err);      /* API_CALL: observed status or -1; TOKEN_EXPIRED: 401; else -1 */

/* AIMUX_E_API_CALL — NULL / -1 under any other code */
int64_t aimux_error_retry_ms(const aimux_error_t *err);         /* retry hint, or -1; 0 = retry now */
char   *aimux_error_provider_code(const aimux_error_t *err);    /* provider's own code, e.g. "insufficient_quota" */
char   *aimux_error_provider_message(const aimux_error_t *err); /* the failure's own text; message is the composed form */
char   *aimux_error_request_id(const aimux_error_t *err);
char   *aimux_error_response_body(const aimux_error_t *err);

/* AIMUX_E_NO_SUCH_MODEL */
char   *aimux_error_model_id(const aimux_error_t *err);
char   *aimux_error_model_type(const aimux_error_t *err);

/* AIMUX_E_NO_SUCH_PROVIDER */
char   *aimux_error_provider_id(const aimux_error_t *err);
```

`code` and `message` answer for every non-`NULL` error. Every other getter
belongs to an AiMuxError code and returns `NULL` / `-1` / `0` under any other
code — read it inside the matching `case`. All returned `char *` are owned by the caller
(`aimux_free_string`); a `NULL` payload string under its own code means the
provider did not send it (`provider_code`, `request_id`, `response_body` are
optional even under `AIMUX_E_API_CALL`). Every getter is `NULL`-safe.

Branch on `retryable`, never on the `status` sentinel: a transport failure and
a missing API key both report `status == -1` and disagree about whether a retry
would help. `retry_ms` is a *hint* that rides along — it is `-1` whenever the
provider advertised no delay (neither a `retry-after` / `retry-after-ms`
response header nor a `retry_after_ms` / `retry_after` member in the JSON
error payload), including on a retryable status, so fall back to your own
exponential backoff when it is negative.

The unified `aimux_error_code_t` keeps the existing AiMuxError values 1–13,
adds RecordingError values 100–105, and assigns C ABI failures 200–206:
`NULL_POINTER`, `INVALID_UTF8`, `INVALID_WIRE_JSON`, `INVALID_HANDLE`,
`REENTRANT_CALL`, `RESULT_SERIALIZATION`, and `CALLBACK_FAILURE`. Values are
never renumbered or reused. A code outside the enum is a header/library
mismatch.

Internal panics abort the process in this workspace's **release** profile
(`panic = "abort"`); in a `panic=unwind` build a Rust callback's panic is
caught inside the C ABI and reported as a C ABI failure instead. Either
way a callback must not unwind across the C ABI — catch your own language's
exceptions inside the callback.

### Recording codes

Only `aimux_init_recording` and `aimux_recording_try_flush` can produce it.

```c
typedef enum aimux_error_code {
    AIMUX_OK = 0,
    AIMUX_E_RECORDING_INIT = 100,
    AIMUX_E_RECORDING_OPEN_FILE = 101,
    AIMUX_E_RECORDING_SPAWN = 102,
    AIMUX_E_RECORDING_WRITER_GONE = 103,
    AIMUX_E_RECORDING_FLUSH_TIMEOUT = 104,
    AIMUX_E_RECORDING_WRITE = 105
} aimux_error_code_t;

aimux_error_t *e = aimux_recording_try_flush();
if (e) {
    char *msg = aimux_error_message(e);
    fprintf(stderr, "recording %d: %s\n", aimux_error_code(e), msg);
    aimux_free_string(msg);
    aimux_error_free(e);
}
```

### Quick start

```c
#include "aimux-error.h"
#include "aimux-ffi.h"

static void die(const char *what, aimux_error_t *e) {
    char *msg = aimux_error_message(e); /* AiMuxError, RecordingError, or C ABI failure */
    fprintf(stderr, "%s: %s\n", what, msg);
    aimux_free_string(msg);
    aimux_error_free(e);
}

uint64_t handle = 0;
aimux_error_t *e = aimux_openai_new("sk-...", "gpt-4o", &handle);
if (e) { die("openai", e); return 1; }

char *result = NULL;
e = aimux_generate_text(handle, "\"What is Rust?\"", "{}", &result);
if (e) { die("generate_text", e); aimux_drop_handle(handle); return 1; }
printf("%s\n", result);
aimux_free_string(result);
aimux_drop_handle(handle);
```

### Branching on code

```c
if (e) {
    int32_t code = aimux_error_code(e);
    if (code >= AIMUX_E_FFI_NULL_POINTER &&
        code <= AIMUX_E_FFI_CALLBACK_FAILURE) {
        /* C ABI failure: this call was wrong — fix the code */
        char *m = aimux_error_message(e);
        fprintf(stderr, "bad call: %s\n", m);
        aimux_free_string(m);
        aimux_error_free(e);
        return 1;
    }
    char *msg = aimux_error_message(e);
    switch (code) {
    case AIMUX_E_API_CALL: {
        /* every HTTP-shaped failure; classify on status */
        int status = aimux_error_status(e);
        if (status == 429) {
            /* rate limited — use aimux_error_retry_ms(e) only if >= 0 (the
               headers may carry no hint), else your own backoff */
        } else if (status == 401) {
            fprintf(stderr, "auth HTTP 401: %s\n", msg);
        }
        char *pc = aimux_error_provider_code(e); /* NULL when the provider sent none */
        if (pc) {
            fprintf(stderr, "provider code: %s\n", pc);
        }
        aimux_free_string(pc);
        break;
    }
    case AIMUX_E_TOKEN_EXPIRED:
        fprintf(stderr, "token expired (HTTP %d): %s\n", aimux_error_status(e), msg);
        break;
    case AIMUX_E_NO_SUCH_PROVIDER: {
        char *id = aimux_error_provider_id(e);
        fprintf(stderr, "no provider \"%s\"\n", id);
        aimux_free_string(id);
        break;
    }
    default:
        fprintf(stderr, "%s\n", msg);
        break;
    }
    aimux_free_string(msg);
    aimux_error_free(e);
}
```

### Streaming

- Success: `NULL` after `on_done`. Terminal failure: non-`NULL` error, `on_done` is not called.
- No `on_error` callback.
- `on_part(json, stream_ctx)` / `on_done(stream_ctx)` — both required; a `NULL`
  callback is a C ABI failure up front (`"on_part: must not be NULL"`)
  instead of being dereferenced.
- A provider `StreamPart::Error` is **data** on `on_part`, not a terminal call failure.
- A part this layer cannot serialize ends the stream with a C ABI failure;
  you never receive a placeholder `{}`.
- Callbacks must not unwind across the ABI (catch C++ exceptions inside the
  trampoline) and must not call back into `aimux_*` (re-entrant call →
  C ABI failure).

```c
static void on_part(const char *json, void *ctx) {
    (void)ctx;
    printf("%s\n", json); /* copy if you need it after return */
}

static void on_done(void *ctx) { (void)ctx; }

aimux_error_t *e = aimux_stream_text(handle, "\"hi\"", NULL, on_part, on_done, NULL);
if (e) {
    char *msg = aimux_error_message(e);
    fprintf(stderr, "%s\n", msg);
    aimux_free_string(msg);
    aimux_error_free(e);
}
```

## Function list

Every fallible symbol returns `aimux_error_t *` (`NULL` = success). The
result, if any, is written to the trailing out-parameter: `uint64_t *out_handle`
for constructors, `char **out_json` for JSON results (free with
`aimux_free_string`). Signatures below are exact; the tag
(`[AiMuxError]` / `[RecordingError]` / `[C ABI]`) identifies the expected
high-level range. Any fallible call can additionally return 200..206.

### Language model

| Function | Description |
|------|------|
| `aimux_openai_new(api_key, model_id, uint64_t *out_handle)` | [AiMuxError] Create an OpenAI language model |
| `aimux_openai_new_with_base(api_key, model_id, base_url, uint64_t *out_handle)` | [AiMuxError] OpenAI with custom base_url |
| `aimux_anthropic_new(api_key, model_id, out_handle)` / `_with_base(…, base_url, out_handle)` | Anthropic |
| `aimux_cohere_new` / `_with_base` | Cohere (same shape) |
| `aimux_mistral_new` / `_with_base` | Mistral |
| `aimux_xai_new` / `_with_base` | xAI |
| `aimux_bedrock_new(access_key_id, secret_access_key, region, model_id, out_handle)` / `_with_base(…, base_url, out_handle)` | Bedrock (AWS SigV4) |
| `aimux_vertex_new(access_token, project, location, model_id, out_handle)` / `_with_base(…, base_url, out_handle)` | Vertex AI |
| `aimux_anthropic_aws_new(api_key, region, model_id, out_handle)` / `_with_base(…, base_url, out_handle)` | Anthropic on AWS |
| `aimux_azure_new(api_key, resource_name, deployment, api_version, out_handle)` / `_with_base(api_key, base_url, deployment, api_version, out_handle)` | Azure OpenAI (`_with_base` requires `base_url`) |
| `aimux_provider_new(name, api_key, model_id, config_json, uint64_t *out_handle)` | [AiMuxError] Registry provider (`api_key` NULL → env) |
| `aimux_provider_from_env(name, model_id, uint64_t *out_handle)` | [AiMuxError] Registry + env API key |
| `aimux_provider_handle_new(name, api_key, config_json, uint64_t *out_handle)` | [AiMuxError] RFC-0027 provider handle |
| `aimux_provider_list_models(handle, char **out_models_json)` | [AiMuxError] JSON `RuntimeModel[]` |
| `aimux_provider_model(handle, model_id, uint64_t *out_handle)` | [AiMuxError] Model from a provider handle |
| `aimux_get_model_specs(source_url, char **out_specs_json)` | [AiMuxError] Community catalogue (`source_url` may be NULL) |
| `aimux_generate_text(handle, prompt_json, opts_json, char **out_json)` | [AiMuxError] Non-streaming (`GenerateTextResult` JSON) |
| `aimux_generate_object(handle, prompt_json, opts_json, char **out_json)` | [AiMuxError] `GenerateObjectResult` JSON |
| `aimux_consume_stream_text(handle, prompt_json, opts_json, char **out_json)` | [AiMuxError] Aggregated stream result JSON |
| `aimux_stream_text(handle, prompt_json, opts_json, on_part, on_done, stream_ctx)` | [AiMuxError] Streaming (push callbacks); no out-param |
| `aimux_stream_text_with_abort(handle, abort_handle, prompt_json, opts_json, on_part, on_done, stream_ctx)` | [AiMuxError] Cancelable stream (`AIMUX_E_ABORTED` on abort) |
| `aimux_generate_text_as_openai(handle, prompt_json, opts_json, char **out_json)` | [AiMuxError] `ChatCompletion` JSON (RFC-0026) |
| `aimux_stream_text_as_openai(…)` / `_with_abort(…)` | [AiMuxError] `ChatCompletionChunk` per part; same shapes as `aimux_stream_text` / `_with_abort` |
| `uint64_t aimux_abort_signal_new(void)` / `void aimux_abort_signal_abort(h)` / `void aimux_abort_signal_drop(h)` | Stream cancellation; infallible |

### Embedding / speech / image / video / rerank / search / files / transcription

Constructors are `[C ABI]` (they only store config); the calls are `[AiMuxError]`.

| Function | Description |
|------|------|
| `aimux_openai_embedding_new` / `aimux_cohere_embedding_new` / `aimux_google_embedding_new` `(api_key, model_id, uint64_t *out_handle)` (+ `_with_base(…, base_url, out_handle)`) | Embedding models |
| `aimux_embed(handle, values_json, opts_json, char **out_json)` | `values_json`: JSON string array |
| `aimux_openai_speech_new(api_key, model_id, out_handle)` / `_with_base` → `aimux_speech_generate(handle, opts_json, char **out_json)` | TTS |
| `aimux_openai_image_new` / `aimux_google_image_new` `(api_key, model_id, out_handle)` / `_with_base` → `aimux_image_generate(handle, opts_json, char **out_json)` | Image |
| `aimux_openai_transcription_new(api_key, model_id, out_handle)` / `_with_base` → `aimux_transcription_generate(handle, audio_base64, media_type, opts_json, char **out_json)` | STT (`opts_json` ignored) |
| `aimux_openai_files_new(api_key, out_handle)` / `_with_base(api_key, base_url, out_handle)` → `aimux_file_upload(handle, data_base64, media_type, opts_json, char **out_json)` | Files (`opts_json` ignored) |
| `aimux_cohere_reranking_new(api_key, model_id, out_handle)` / `_with_base` → `aimux_rerank(handle, opts_json, char **out_json)` | Rerank |
| `aimux_google_video_new(api_key, model_id, out_handle)` / `_with_base` → `aimux_video_generate(handle, opts_json, char **out_json)` | Video |
| `aimux_tavily_search_new(api_key, model_id, out_handle)` / `_with_base` → `aimux_search(handle, opts_json, char **out_json)` | Search (`model_id` ignored) |
| `aimux_codex_refresh(refresh_token, client_id, char **out_json)` | [AiMuxError] Codex OAuth refresh (RFC-0018) |

`aimux_speech_generate`, `aimux_image_generate`, `aimux_rerank`,
`aimux_video_generate` and `aimux_search` **require** a JSON object in
`opts_json` — it carries the input, not just options; NULL or empty is a
C ABI failure. `aimux_embed`'s `opts_json` is optional, and
`aimux_transcription_generate` / `aimux_file_upload` ignore theirs.

### Transcription streaming (RFC-0028)

| Function | Description |
|------|------|
| `aimux_transcription_session_new(model_handle, abort_handle, opts_json, uint64_t *out_handle)` | [AiMuxError] Start a session (`abort_handle` 0 = none) |
| `aimux_transcription_push_audio(session, const uint8_t *data, size_t len)` | [AiMuxError] Push a chunk (blocking on backpressure) |
| `aimux_transcription_input_done(session)` | [C ABI] End of audio (idempotent) |
| `aimux_transcription_next_part(session, int64_t timeout_ms, char **out_part, int32_t *out_state)` | [AiMuxError] Pull; on `NULL` return `*out_state` is `PART` (`*out_part` set) / `ENDED` / `TIMEOUT`. `timeout_ms`: >0 wait, 0 poll, <0 forever |
| `void aimux_transcription_session_drop(session)` | Terminate and release |

A pull timeout is a **state**, not an error.

### Resource management

| Function | Description |
|------|------|
| `void aimux_drop_handle(handle)` | Free a handle (`0` is a no-op) |
| `void aimux_free_string(ptr)` | Free a result / getter string (`NULL` is safe) |
| `void aimux_error_free(err)` | Release a returned error (`NULL` is safe); never `aimux_drop_handle` / `aimux_free_string` |
| `aimux_error_code` / `aimux_error_message` / `aimux_error_*` | Read an error (see aimux-error.h) |

### Session (RFC-0024)

| Function | Description |
|------|------|
| `void aimux_session_store_init(void)` | Register global session store |
| `void aimux_session_infer_init(int32_t enabled)` | Opt-in session inferer |
| `aimux_session_calls(session_id, char **out_json)` | [C ABI] JSON `SessionCall[]` |
| `aimux_list_sessions(char **out_json)` | [C ABI] JSON `SessionView[]` |

Pass `"session_id": "..."` inside `opts_json` of generate/stream to group a call.

### Cache probing (RFC-0015)

| Function | Description |
|------|------|
| `aimux_trace_new(handle, uint64_t *out_handle)` | [C ABI] Probe wrapper handle |
| `aimux_trace_new_audited(handle, int32_t strict, uint64_t *out_handle)` | [C ABI] With rules auditor |
| `aimux_trace_aggregate(handle, filter_json, char **out_json)` | [AiMuxError] JSON `TraceStats[]` (`filter_json` NULL is a C ABI failure) |
| `aimux_trace_session_chain(handle, session_id, char **out_json)` | [AiMuxError] JSON `SessionChainView`; unknown session → `AIMUX_E_INVALID_ARGUMENT` |
| `aimux_trace_session_trajectory(handle, session_id, char **out_json)` | [C ABI] Per-step stats JSON |
| `aimux_trace_export_jsonl(handle, char **out_jsonl)` | [C ABI] JSONL export |
| `aimux_trace_clear(handle)` | [C ABI] Clear records; fails only for a dead handle |

### Configuration

| Function | Description |
|------|------|
| `aimux_init_logging(level)` | [C ABI] `"off"…"trace"`, NULL = `"warn"` |
| `aimux_register_providers(config_json)` | [AiMuxError] External OpenAI-compatible providers (RFC-0020) |
| `aimux_init_proxy(config_json)` | [AiMuxError] Global proxy (before first call) |
| `aimux_router_new(const uint64_t *handles, size_t len, config_json, uint64_t *out_handle)` | [AiMuxError] RouterModel (RFC-0021) |
| `aimux_moa_new(const uint64_t *reference_handles, size_t ref_len, aggregator, config_json, uint64_t *out_handle)` | [AiMuxError] MoaModel (RFC-0022) |

### Recording (RFC-0023)

| Function | Description |
|------|------|
| `aimux_init_recording(dir)` | [RecordingError] Opt-in JSONL recording. NULL / non-UTF-8 `dir` is a C ABI failure; recorder construction failure is code 100 / 101 / 102 |
| `aimux_init_recording_ring(uint64_t cap)` | [AiMuxError] In-memory ring; `cap == 0` → `AIMUX_E_INVALID_ARGUMENT` |
| `void aimux_init_recording_ring_default(void)` | Ring with default capacity (2048) |
| `void aimux_recording_stop(void)` / `void aimux_recording_flush(void)` | Stop / flush (write failures not reported) |
| `aimux_recording_try_flush(void)` | [RecordingError] Flush; `NULL` = on disk, else code 105 / 103 / 104 |
| `aimux_mock_replay_new(recordings_jsonl, uint64_t *out_handle)` | [AiMuxError] Replay model handle |

## Examples

### Video

```c
uint64_t handle = 0;
aimux_error_t *e = aimux_google_video_new(api_key, "veo-3.0", &handle);
if (e) { /* aimux_error_code(e); aimux_error_message(e); aimux_error_free(e) */ }
char *result = NULL;
e = aimux_video_generate(handle, opts_json, &result);
if (e) { /* aimux_error_code(e) …; aimux_error_free(e) */ }
aimux_free_string(result);
aimux_drop_handle(handle);
```

### Reranking / search

Same pattern: `*_new(..., &handle)`, then `aimux_rerank` / `aimux_search` with `&result`.

## Memory management

- Result `char *` written to `*out_json`: free with `aimux_free_string`.
- Stream callback `const char *`: valid only for the duration of the callback; copy if needed.
- Returned error: strings from `aimux_error_*` getters are freed with `aimux_free_string`; release the error itself with `aimux_error_free` exactly once.
- `aimux_drop_handle(0)`, `aimux_free_string(NULL)`, `aimux_error_free(NULL)` are no-ops.

## Headers

- `aimux-ffi/aimux-ffi.h` — full C ABI (`extern "C"` for C++)
- `aimux-ffi/aimux-error.h` — included by `aimux-ffi.h`; can be included alone for the error types
