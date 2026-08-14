# aimux · C ABI (C/C++)

> The C ABI boundary (`aimux-ffi`) is shared by Swift / Kotlin / Flutter / Go / Java / C++.
> **Results** are JSON strings. **Errors** are not JSON: they use return-value
> sentinels plus an optional `AimuxError *` out-parameter (see
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
- [aimux-error.h](../../aimux-ffi/aimux-error.h) — `AimuxError` / `AimuxErrorCode`

## Error handling

### How to tell failure

**Only the return value decides success or failure.**

| Return type | Success | Failure |
|-------------|---------|---------|
| Constructor / handle (`uint64_t`) | `≥ 1` | **`0`** |
| Payload (`char *` JSON) | non-`NULL` | **`NULL`** |
| Stream (`int32_t`) | non-zero (e.g. `1`) | **`0`** |

```c
if (!result) {
    /* failed — then read *err if you passed one */
}
```

Do **not** sniff JSON for `"error"` or rely on `err` alone without checking the return value.

### Optional details: `AimuxError *err`

Every fallible call takes a trailing `AimuxError *err` (may be `NULL`).

| | |
|--|--|
| Failure + `err != NULL` | Callee fills `*err` (`code != AIMUX_OK`, non-empty `message`) |
| Failure + `err == NULL` | Still fails; no details (caller discarded them) |
| Success | Use the return value; `*err` is **not touched** |

The struct is 40 bytes (four fields plus two reserved pointer slots for future ABI extension); on failure `message` is allocated by aimux and the
caller **must release it with `aimux_free_string`** after reading. Initialize
with `aimux_error_clear` (or `= {0}`) before first use.

```c
typedef struct AimuxError {
    AimuxErrorCode code;   /* switch on this */
    int status;            /* HTTP status, or -1 */
    int64_t retry_ms;      /* ApiCall retry hint (retry_after_ms), or -1; 0 = retry now */
    char *message;         /* owned; free with aimux_free_string */
    char *error_value;     /* owned; lossless AiMuxError JSON, or NULL */
    void *reserved[1];     /* future ABI room; always zero */
} AimuxError;
```

`error_value` carries the machine-readable source error — the
externally-tagged JSON of aimux-core's `AiMuxError`, e.g.
`{"ApiCall":{"status_code":429,"retry_after_ms":1500,"message":"quota"}}`. It is NULL when
the failure was synthesized at the FFI boundary (bad argument, invalid
handle). Free it with `aimux_free_string` like `message`.

Internal panics abort the process (the workspace builds with `panic=abort`).

Codes: `AIMUX_OK`, `AIMUX_E_UNKNOWN`, plus **13** values mirroring
`aimux-core::AiMuxError` (`AIMUX_E_JSON_PARSE=2` … `AIMUX_E_OTHER=14`,
numbered consecutively). The enum is
append-only; always handle `default` / `AIMUX_E_UNKNOWN`.

### Quick start

```c
#include "aimux-error.h"
#include "aimux-ffi.h"

AimuxError err;
aimux_error_clear(&err);
uint64_t handle = aimux_openai_new("sk-...", "gpt-4o", &err);
if (!handle) {
    fprintf(stderr, "%s\n", err.message);
    aimux_free_string(err.message);
    aimux_free_string(err.error_value);
    return 1;
}

char *result = aimux_generate_text(handle, "\"What is Rust?\"", "{}", &err);
if (!result) {
    fprintf(stderr, "%s\n", err.message);
    aimux_free_string(err.message);
    aimux_free_string(err.error_value);
    aimux_drop_handle(handle);
    return 1;
}
printf("%s\n", result);
aimux_free_string(result);
aimux_drop_handle(handle);
```

### Branching on code

```c
if (!handle) {
    switch (err.code) {
    case AIMUX_E_API_CALL:
        /* every HTTP-shaped failure; classify on err.status */
        if (err.status == 429) {
            /* rate limited — use err.retry_ms */
        } else if (err.status == 401) {
            fprintf(stderr, "auth HTTP 401: %s\n", err.message);
        }
        break;
    case AIMUX_E_TOKEN_EXPIRED:
        fprintf(stderr, "token expired (HTTP %d): %s\n", err.status, err.message);
        break;
    default:
        fprintf(stderr, "%s\n", err.message);
        break;
    }
    aimux_free_string(err.message);
    aimux_free_string(err.error_value);
}
```

### Streaming

- Terminal failure: return `0`, fill `err` if non-NULL; `on_done` is not called.
- No `on_error` callback.
- `on_part(json, stream_ctx)` / `on_done(stream_ctx)`.
- A provider `StreamPart::Error` is **data** on `on_part`, not a terminal call failure.

```c
static void on_part(const char *json, void *ctx) {
    (void)ctx;
    printf("%s\n", json); /* copy if you need it after return */
}

static void on_done(void *ctx) { (void)ctx; }

AimuxError err;
aimux_error_clear(&err);
if (!aimux_stream_text(handle, "\"hi\"", NULL, on_part, on_done, NULL, &err)) {
    fprintf(stderr, "%s\n", err.message);
    aimux_free_string(err.message);
    aimux_free_string(err.error_value);
}
```

## Function list

Unless noted, fallible symbols take a trailing `AimuxError *err` (may be `NULL`).
Constructors return `uint64_t` (`0` = failure). Payload calls return `char *`
(`NULL` = failure). Streams return `int32_t` (`0` = failure, non-zero = success).

### Language model

| Function | Description |
|------|------|
| `aimux_openai_new(api_key, model_id, err)` | Create an OpenAI language model |
| `aimux_openai_new_with_base(api_key, model_id, base_url, err)` | OpenAI with custom base_url |
| `aimux_anthropic_new` / `_with_base` | Anthropic |
| `aimux_cohere_new` / `_with_base` | Cohere |
| `aimux_mistral_new` / `_with_base` | Mistral |
| `aimux_xai_new` / `_with_base` | xAI |
| `aimux_bedrock_new` / `_with_base` | Bedrock (AWS SigV4) |
| `aimux_vertex_new` / `_with_base` | Vertex AI |
| `aimux_anthropic_aws_new` / `_with_base` | Anthropic on AWS |
| `aimux_azure_new` / `_with_base` | Azure OpenAI |
| `aimux_provider_new(name, api_key, model_id, config_json, err)` | Registry provider (`api_key` NULL → env) |
| `aimux_provider_from_env(name, model_id, err)` | Registry + env API key |
| `aimux_provider_handle_new` / `aimux_provider_list_models` / `aimux_provider_model` | RFC-0027 provider handle |
| `aimux_generate_text(handle, prompt_json, opts_json, err)` | Non-streaming (JSON result string) |
| `aimux_stream_text(handle, prompt_json, opts_json, on_part, on_done, stream_ctx, err)` | Streaming (push callbacks) |
| `aimux_generate_text_as_openai` / `aimux_stream_text_as_openai` / `_with_abort` | OpenAI-compatible shapes (RFC-0026) |
| `aimux_abort_signal_new` / `_abort` / `_drop` | Stream cancellation |

### Embedding / speech / image / video / rerank / search / files

Same pattern: `*_new(..., err)` → handle; `aimux_embed` / `aimux_speech_generate` /
`aimux_image_generate` / `aimux_video_generate` / `aimux_rerank` / `aimux_search` /
`aimux_transcription_generate` / `aimux_file_upload` → `char *` or failure `NULL`.

### Resource management

| Function | Description |
|------|------|
| `aimux_drop_handle(handle)` | Free a handle (`0` is a no-op); no `err` |
| `aimux_free_string(ptr)` | Free a result string (`NULL` is safe) |

### Session (RFC-0024)

| Function | Description |
|------|------|
| `aimux_session_store_init()` | Register global session store |
| `aimux_session_infer_init(enabled)` | Opt-in session inferer |
| `aimux_session_calls(session_id, err)` | JSON `SessionCall[]` |
| `aimux_list_sessions(err)` | JSON `SessionView[]` |

Pass `"session_id": "..."` inside `opts_json` of generate/stream to group a call.

### Cache probing (RFC-0015)

| Function | Description |
|------|------|
| `aimux_trace_new(handle, err)` | Probe wrapper handle |
| `aimux_trace_new_audited(handle, strict, err)` | With rules auditor |
| `aimux_trace_aggregate(handle, filter_json, err)` | JSON `TraceStats[]` |
| `aimux_trace_session_chain(handle, session_id, err)` | JSON `SessionChainView` |
| `aimux_trace_session_trajectory(handle, session_id, err)` | Per-step stats JSON |
| `aimux_trace_export_jsonl(handle, err)` | JSONL export |
| `aimux_trace_clear(handle)` | Clear records (returns `int`, no `err`) |

Legacy `int`-returning utilities (`aimux_init_logging`, `aimux_session_*_init`,
`aimux_trace_clear`, `aimux_recording_*`) keep `0 = success / -1 = failure` —
the opposite polarity of streams; they take no `err`.

### Recording (RFC-0023)

| Function | Description |
|------|------|
| `aimux_init_recording(dir)` / `aimux_init_recording_ring(cap)` | Opt-in recording |
| `aimux_recording_stop` / `aimux_recording_flush` | Stop / flush |
| `aimux_recording_try_flush` | Flush with write-failure reporting |
| `aimux_mock_replay_new(recordings_jsonl, err)` | Replay model handle |

## Examples

### Video

```c
AimuxError err;
uint64_t handle = aimux_google_video_new(api_key, "veo-3.0", &err);
if (!handle) { /* err */ }
char *result = aimux_video_generate(handle, opts_json, &err);
if (!result) { /* err */ }
aimux_free_string(result);
aimux_drop_handle(handle);
```

### Reranking / search

Same pattern: `*_new(..., &err)`, then `aimux_rerank` / `aimux_search` with `&err`.

## Memory management

- Result `char *` from generate/embed/…: free with `aimux_free_string`.
- Stream callback `const char *`: valid only for the duration of the callback; copy if needed.
- `AimuxError`: caller storage; on failure free `err.message` and `err.error_value` with `aimux_free_string`.
- `aimux_drop_handle(0)` is a no-op.

## Headers

- `aimux-ffi/aimux-ffi.h` — full C ABI (`extern "C"` for C++)
- `aimux-ffi/aimux-error.h` — included by `aimux-ffi.h`; can be included alone for the error types
