# aimux · C ABI (C/C++)

> **0.6.0.** The C ABI is ABI 1 / ops 1.0. The 0.5 provider exports are
> removed and there are no forwarding aliases. See the
> [0.6 upgrade guide](../upgrading-to-0.6.md) and the
> [ops ABI migration](../upgrading-to-ops-abi.md).

The C ABI is shared by Swift, Kotlin, Flutter, Go, Java and C/C++. It exposes
14 primitives; operation names, versions and the manifest digest come from the
generated header. Results are JSON strings in a trailing out-parameter. Errors
are one owned `aimux_error_t *` (`NULL` means success).

## Install

Download the matching platform library and `aimux.h` from
[GitHub Releases](https://github.com/arcships/aimux/releases):

| Platform | Library |
|---|---|
| Linux x86_64 | `libaimux_ffi-linux-x64.so` |
| macOS arm64 | `libaimux_ffi-macos-arm64.dylib` |
| Windows x86_64 | `aimux_ffi-windows-x64.dll` |

Android ABI libraries and the iOS XCFramework are also published. Link against
the library and include the generated header:

```bash
gcc -o example example.c -I. -L. -laimux_ffi -lpthread -ldl -lm
```

## Quick start

```c
#include "aimux.h"

char *manifest = NULL;
aimux_error_t *err = aimux_manifest_json(&manifest);
if (err) { /* handle */ }
puts(manifest);
aimux_free_string(manifest);

const char *provider_params =
    "{\"package\":\"openai\",\"settings\":{\"apiKey\":\"sk-...\"}}";
char *provider_json = NULL;
err = aimux_call(0, AIMUX_OP_PROVIDER_CREATE, provider_params, 0, &provider_json);
if (err) { /* handle */ }

/* provider_json is {"handle":"<decimal>"}; parse it with your JSON parser. */
uint64_t provider = /* parsed handle */ 0;
aimux_free_string(provider_json);

const char *model_params =
    "{\"method\":\"languageModel\",\"modelId\":\"gpt-4o-mini\"}";
char *model_json = NULL;
err = aimux_call(provider, AIMUX_OP_PROVIDER_MODEL, model_params, 0, &model_json);
if (err) { /* handle */ }

/* model_json is {"handle":"<decimal>"}; parse it with your JSON parser. */
uint64_t model = /* parsed handle */ 0;
aimux_free_string(model_json);

const char *request =
    "{\"prompt\":\"Hello\",\"options\":{}}";
char *result = NULL;
err = aimux_call(model, AIMUX_OP_TEXT_GENERATE, request, 0, &result);
if (err) { /* handle */ }

puts(result);
aimux_free_string(result);
aimux_drop(model);
aimux_drop(provider);
```

Handle 0 is global scope; nonzero IDs are process-owned and never reused.
Handles are `uint64_t` values but travel as decimal strings inside JSON. Never
adopt the same ID twice. `aimux_drop` is idempotent and cancels resources; it
does not flush a recorder.

## Streams

```c
static void on_event(const char *frame_json, void *context) {
    /* frame_json is borrowed until this callback returns; copy to retain. */
    puts(frame_json);
}

err = aimux_stream(model, AIMUX_OP_TEXT_STREAM, request, 0, on_event, NULL);
```

`aimux_stream` returns after the producer finishes. A stream has business event
frames and one explicit end frame. Callbacks must return promptly; a blocking
call cannot be reentered from a callback (`ReentrantCall`). Pass a live abort
handle for cancellation.

For transcription, use `aimux_operation_push` and
`aimux_operation_next_part`. `aimux_operation_next_part` returns `PART`,
`ENDED` or `TIMEOUT`; only `PART` allocates an output string.

## Errors and ownership

```c
if (err) {
    char *json = aimux_error_json(err);
    fprintf(stderr, "code=%d message=%s json=%s\n",
            aimux_error_code(err), aimux_error_message(err), json);
    aimux_free_string(json);
    aimux_error_free(err);
}
```

- `NULL` means success; a non-NULL error is owned by the caller and released
  exactly once with `aimux_error_free`.
- `aimux_error_json` returns the full envelope, including `error.cause`.
- Every output JSON string and error getter string is freed with
  `aimux_free_string`.
- Do not free callback JSON.
- `aimux_abi_version()` and `AIMUX_MANIFEST_DIGEST` let a consumer verify the
  loaded library before use.

## Breaking changes in 0.6

- `aimux_openai_new`, `aimux_anthropic_new` and the other provider-specific
  exports are removed.
- The ops ABI replaces the unversioned function surface; operation names come
  from `AIMUX_OP_*`.
- Error transport is one opaque `aimux_error_t *`; payload-specific C getters
  are gone. Read the envelope with `aimux_error_json`.
- Provider/model creation is `aimux_call(..., AIMUX_OP_PROVIDER_CREATE, ...)`
  followed by `AIMUX_OP_PROVIDER_MODEL`.
- The generated header and manifest digest must match the library.

## See also

- [bindings/c/README.md](../../bindings/c/README.md) — examples, ASan/LSan
  harnesses and ownership tests.
- [upgrading-to-ops-abi.md](../upgrading-to-ops-abi.md) — old C export mapping.
- [docs/api/ops.md](ops.md) — operation names.
- [docs/error-model.md](../error-model.md) — error codes and cause chains.
- [C provider list](providers.md).
