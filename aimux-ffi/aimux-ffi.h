/**
 * aimux-ffi.h — C ABI for aimux multi-language bindings.
 *
 * This C ABI is used by C / C++ and the Go, Kotlin, Java, Swift, and Flutter
 * bindings. Native bindings (Python / Node)
 * bypass this layer and use aimux-providers directly.
 *
 * ## Errors
 *
 * Every fallible function returns `aimux_error_t *`: NULL on success
 * (the result is in the out-parameter), non-NULL on failure (the out-parameter
 * remains at its sentinel: handle 0, pointer NULL). Every non-NULL error has
 * one code and one message, read with `aimux_error_code()` and
 * `aimux_error_message()`. Release it exactly once with `aimux_error_free()`.
 * Getter strings are caller-owned (`aimux_free_string`). See aimux-error.h.
 *
 * Each prototype below identifies its expected high-level error:
 *   [AiMuxError]      codes 1..14
 *   [RecordingError]  codes 100..105
 *   [C ABI]           no expected high-level code
 * Every fallible call can additionally return a C ABI failure (200..206).
 *
 * Functions with no failure path keep their natural signature (`void`, or a
 * plain value such as the `uint64_t` of aimux_abort_signal_new).
 *
 * ## Memory ownership
 *
 * Handles (`uint64_t`, non-zero) are released with aimux_drop_handle — every
 * handle category, including transcription sessions (whose driver task is aborted and
 * joined; aimux_transcription_session_drop is the same operation under a
 * clearer name).
 * Result strings (`char *` written to `*out_json`) are freed with
 * aimux_free_string (NULL is safe). Returned errors are released with
 * aimux_error_free.
 * aimux_stream_text callbacks receive `const char*` pointers that are valid
 * **only for the duration of the callback**. The callback must copy the data
 * synchronously.
 *
 * ## Callbacks
 *
 * Callbacks must not unwind across the Aimux C ABI. A Rust callback's
 * unwinding panic (panic=unwind builds) is caught and reported as a C ABI
 * failure; foreign exceptions (C++, JVM, Swift, Dart, Go) must be caught
 * inside the callback trampoline. Callbacks execute on the calling thread; a
 * blocking aimux_* call made from inside a callback is rejected as a C ABI
 * failure (re-entrant call) rather than deadlocking the tokio runtime.
 *
 * ## Concurrency
 *
 * All functions are synchronous (block until the operation completes).
 * `opts_json.timeout` (`{"total_ms","step_ms","first_chunk_ms","chunk_ms"}`,
 * milliseconds; the latter two streaming-only) bounds a call from the callee
 * side; `aimux_abort_signal_*` plus the `*_with_abort` entry points cancel a
 * running call from another thread.
 *
 * ## Wire format
 *
 * - Internal panics abort the process (the workspace builds with panic=abort).
 * - `prompt_json`: bare prompt value (`"text"` or `[{...}]`), or a
 *   single-key wrapper `{"prompt": <value>}` (any extra key disables
 *   unwrapping and the whole object is parsed as the prompt)
 * - `opts_json` for `aimux_generate_text`/`aimux_stream_text`: serialized
 *   GenerateTextOptions (NULL / empty / "null" for defaults). Multimodal
 *   calls (`aimux_speech_generate`, `aimux_image_generate`, `aimux_rerank`,
 *   `aimux_video_generate`, `aimux_search`) REQUIRE a valid JSON object —
 *   NULL or empty is a C ABI failure.
 * - Malformed JSON text is a C ABI failure; well-formed JSON of the wrong
 *   shape is AiMuxError AIMUX_E_INVALID_ARGUMENT.
 * - Results: serialized JSON of GenerateTextResult on success
 * - Stream parts: serialized JSON of StreamPart
 */

#ifndef AIMUX_FFI_H
#define AIMUX_FFI_H

#include <stddef.h>
#include <stdint.h>

#include "aimux-error.h"

#ifdef __cplusplus
extern "C" {
#endif

/* ── Provider constructors ──────────────────────────────────────────────── */
/* [AiMuxError] Every constructor writes a non-zero handle to *out_handle.
   AiMuxError: invalid model id / config. C ABI failure: NULL or non-UTF-8
   argument. `_with_base` adds `base_url` (NULL or empty uses the provider
   default). */

/**
 * Create an OpenAI model instance.
 *
 * @param api_key    NUL-terminated API key string.
 * @param model_id   NUL-terminated model ID (e.g. "gpt-4o").
 * @param out_handle Receives the model handle.
 */
aimux_error_t *aimux_openai_new(const char *api_key, const char *model_id,
                                    uint64_t *out_handle);
aimux_error_t *aimux_openai_new_with_base(const char *api_key, const char *model_id,
                                              const char *base_url, uint64_t *out_handle);

/** Create an Anthropic model instance (e.g. "claude-3-5-sonnet-20241022"). */
aimux_error_t *aimux_anthropic_new(const char *api_key, const char *model_id,
                                       uint64_t *out_handle);
aimux_error_t *aimux_anthropic_new_with_base(const char *api_key, const char *model_id,
                                                 const char *base_url, uint64_t *out_handle);

/** Create a Cohere model instance (API key + model ID). */
aimux_error_t *aimux_cohere_new(const char *api_key, const char *model_id,
                                    uint64_t *out_handle);
aimux_error_t *aimux_cohere_new_with_base(const char *api_key, const char *model_id,
                                              const char *base_url, uint64_t *out_handle);

/** Create a Mistral model instance (API key + model ID). */
aimux_error_t *aimux_mistral_new(const char *api_key, const char *model_id,
                                     uint64_t *out_handle);
aimux_error_t *aimux_mistral_new_with_base(const char *api_key, const char *model_id,
                                               const char *base_url, uint64_t *out_handle);

/** Create an xAI model instance (API key + model ID). */
aimux_error_t *aimux_xai_new(const char *api_key, const char *model_id,
                                 uint64_t *out_handle);
aimux_error_t *aimux_xai_new_with_base(const char *api_key, const char *model_id,
                                           const char *base_url, uint64_t *out_handle);

/** Create a Bedrock model instance (AWS SigV4 credentials). */
aimux_error_t *aimux_bedrock_new(const char *access_key_id,
                                     const char *secret_access_key,
                                     const char *region,
                                     const char *model_id, uint64_t *out_handle);
aimux_error_t *aimux_bedrock_new_with_base(const char *access_key_id,
                                               const char *secret_access_key,
                                               const char *region,
                                               const char *model_id,
                                               const char *base_url, uint64_t *out_handle);

/** Create a Vertex AI model instance (GCP bearer token). */
aimux_error_t *aimux_vertex_new(const char *access_token,
                                    const char *project,
                                    const char *location,
                                    const char *model_id, uint64_t *out_handle);
aimux_error_t *aimux_vertex_new_with_base(const char *access_token,
                                              const char *project,
                                              const char *location,
                                              const char *model_id,
                                              const char *base_url, uint64_t *out_handle);

/** Create an Anthropic-on-AWS model instance (API key + region). */
aimux_error_t *aimux_anthropic_aws_new(const char *api_key, const char *region,
                                           const char *model_id, uint64_t *out_handle);
aimux_error_t *aimux_anthropic_aws_new_with_base(const char *api_key, const char *region,
                                                     const char *model_id, const char *base_url,
                                                     uint64_t *out_handle);

/**
 * Create an Azure OpenAI model instance (API key + resource name; deployment
 * passed as model_id; api_version NULL or empty uses the provider default).
 *
 * `_with_base` takes an explicit `base_url` IN PLACE OF `resource_name`.
 * Unlike other `_with_base` variants, `base_url` here is REQUIRED — NULL is
 * a C ABI failure ("base_url: must not be NULL").
 */
aimux_error_t *aimux_azure_new(const char *api_key, const char *resource_name,
                                   const char *deployment, const char *api_version,
                                   uint64_t *out_handle);
aimux_error_t *aimux_azure_new_with_base(const char *api_key, const char *base_url,
                                             const char *deployment, const char *api_version,
                                             uint64_t *out_handle);

/**
 * Create a model from the provider registry by name (RFC-0017 phase 4).
 *
 * @param name        NUL-terminated registry provider name (e.g. "deepseek", "groq").
 * @param api_key     NUL-terminated API key; NULL reads the provider's env
 *                    var from the registry entry. Non-NULL but non-UTF-8 is
 *                    a C ABI failure.
 * @param model_id    NUL-terminated model ID.
 * @param config_json Optional JSON object of ProviderOptions
 *                    ({"base_url": "...", "headers": {...}, "max_retries": 0,
 *                     "body_overrides": {...}});
 *                    NULL / empty / "null" for defaults.
 * @param out_handle  Receives the model handle. AiMuxError: unknown
 *                    provider, bad config, missing env key, invalid model id.
 */
aimux_error_t *aimux_provider_new(const char *name, const char *api_key,
                                      const char *model_id, const char *config_json,
                                      uint64_t *out_handle);

/**
 * Convenience: create a model by provider name, reading the API key from the
 * provider's env var.
 */
aimux_error_t *aimux_provider_from_env(const char *name, const char *model_id,
                                           uint64_t *out_handle);

/**
 * Provider handles (RFC-0027): create a provider handle for a registry-backed
 * provider. Unlike aimux_provider_new (which binds to a single model_id),
 * this returns a provider handle supporting aimux_provider_list_models and
 * aimux_provider_model.
 */
aimux_error_t *aimux_provider_handle_new(const char *name, const char *api_key,
                                             const char *config_json, uint64_t *out_handle);

/**
 * [AiMuxError] List models on a provider handle (RFC-0027 runtime discovery).
 * Writes a JSON array of sparse RuntimeModel (id / owned_by / created) —
 * no community enrichment. To supplement with model specs, call
 * aimux_get_model_specs separately and merge in the host.
 */
aimux_error_t *aimux_provider_list_models(uint64_t handle, char **out_models_json);

/** [AiMuxError] Build a language model from a provider handle + model_id. */
aimux_error_t *aimux_provider_model(uint64_t handle, const char *model_id,
                                        uint64_t *out_handle);

/**
 * [AiMuxError] Fetch the community model catalogue (anya2a). Writes a
 * JSON-serialized Catalogue (provider → model_id → ModelSpec). source_url may
 * be NULL for the default endpoint. Thin fetch — no caching.
 */
aimux_error_t *aimux_get_model_specs(const char *source_url, char **out_specs_json);

/* ── Generation ─────────────────────────────────────────────────────────── */

/**
 * [AiMuxError] Non-streaming text generation.
 *
 * @param handle      Language-model handle from aimux_*_new
 *                    (a handle of another modality is a C ABI failure).
 * @param prompt_json JSON prompt (see wire format above).
 * @param opts_json   JSON options (NULL or empty for defaults).
 * @param out_json    Receives the serialized GenerateTextResult (caller MUST
 *                    free with aimux_free_string). Malformed
 *                    prompt_json/opts_json fails before any network call.
 */
aimux_error_t *aimux_generate_text(uint64_t handle,
                                       const char *prompt_json,
                                       const char *opts_json, char **out_json);

/* [AiMuxError] Generate a structured JSON object (M12, RFC-0016). Same signature
   as aimux_generate_text; writes serialized GenerateObjectResult JSON.
   Pass response_format: { "Json": { ... } } via opts_json for schema control. */
aimux_error_t *aimux_generate_object(uint64_t handle,
                                         const char *prompt_json,
                                         const char *opts_json, char **out_json);

/* [AiMuxError] Consume a stream to completion and write the aggregated result
   (M11, RFC-0016). Synchronous (blocks until the stream finishes). Same
   signature as aimux_generate_text; writes serialized
   StreamTextResultAggregated JSON. */
aimux_error_t *aimux_consume_stream_text(uint64_t handle,
                                             const char *prompt_json,
                                             const char *opts_json, char **out_json);

/**
 * [AiMuxError] Streaming text generation with push callbacks (blocks until the
 * stream ends).
 *
 * @param handle      Model handle from aimux_*_new.
 * @param prompt_json JSON prompt (see wire format above).
 * @param opts_json   JSON options (NULL or empty for defaults).
 * @param on_part     Called for each StreamPart (JSON string, valid during
 *                    the call only). Required: NULL is a C ABI failure
 *                    ("on_part: must not be NULL").
 * @param on_done     Called once when the stream ends normally. Required.
 * @param stream_ctx  Opaque pointer passed through to on_part / on_done.
 * @return NULL after on_done; on failure an error and no on_done
 *         (provider failure → AiMuxError view; a part that cannot be serialized
 *         → C ABI failure).
 */
aimux_error_t *aimux_stream_text(uint64_t handle,
                                     const char *prompt_json,
                                     const char *opts_json,
                                     void (*on_part)(const char *json, void *stream_ctx),
                                     void (*on_done)(void *stream_ctx),
                                     void *stream_ctx);

/**
 * Create a per-call abort signal. Infallible.
 *
 * @return A non-zero abort handle. Release it with aimux_abort_signal_drop.
 */
uint64_t aimux_abort_signal_new(void);

/**
 * Request cancellation. Invalid handles and repeated calls are safe.
 *
 * @param abort_handle Handle from aimux_abort_signal_new.
 */
void aimux_abort_signal_abort(uint64_t abort_handle);

/**
 * Release an abort handle. Active calls keep their signal alive.
 *
 * @param abort_handle Handle from aimux_abort_signal_new. Zero is safe.
 */
void aimux_abort_signal_drop(uint64_t abort_handle);

/**
 * [AiMuxError] Streaming text generation with per-call cancellation.
 *
 * This function blocks until the stream ends. Another thread can call
 * aimux_abort_signal_abort while this function runs. Cancellation fails the
 * call with AiMuxError AIMUX_E_ABORTED and does not call on_done.
 *
 * @param handle       Model handle from aimux_*_new.
 * @param abort_handle Handle from aimux_abort_signal_new.
 * @param prompt_json  JSON prompt.
 * @param opts_json    JSON options. NULL or empty uses defaults.
 * @param on_part      Called for each StreamPart.
 * @param on_done      Called once after normal completion.
 * @param stream_ctx   Opaque pointer passed through to callbacks.
 */
aimux_error_t *aimux_stream_text_with_abort(uint64_t handle, uint64_t abort_handle,
                                                const char *prompt_json,
                                                const char *opts_json,
                                                void (*on_part)(const char *json, void *stream_ctx),
                                                void (*on_done)(void *stream_ctx),
                                                void *stream_ctx);

/* ── OpenAI-compatible output (RFC-0026) ───────────────────────────────── */

/**
 * [AiMuxError] Non-streaming text generation with OpenAI Chat Completions output.
 *
 * Same as aimux_generate_text, but writes a serialized ChatCompletion
 * (OpenAI "chat.completion" object). Works with any provider.
 */
aimux_error_t *aimux_generate_text_as_openai(uint64_t handle,
                                                 const char *prompt_json,
                                                 const char *opts_json, char **out_json);

/**
 * [AiMuxError] Streaming text generation with OpenAI Chat Completions output.
 *
 * Same as aimux_stream_text, but each on_part receives a serialized
 * ChatCompletionChunk (OpenAI "chat.completion.chunk" object).
 * Works with any provider.
 *
 * @param opts_json May carry providerOptions.openai.stream_options with
 *                  include_usage (bool, default true) and
 *                  include_reasoning (bool, default true).
 */
aimux_error_t *aimux_stream_text_as_openai(uint64_t handle,
                                               const char *prompt_json,
                                               const char *opts_json,
                                               void (*on_part)(const char *json, void *stream_ctx),
                                               void (*on_done)(void *stream_ctx),
                                               void *stream_ctx);

/**
 * [AiMuxError] Cancelable streaming OpenAI-compatible output (see
 * aimux_stream_text_with_abort).
 */
aimux_error_t *aimux_stream_text_as_openai_with_abort(uint64_t handle, uint64_t abort_handle,
                                                          const char *prompt_json,
                                                          const char *opts_json,
                                                          void (*on_part)(const char *json, void *stream_ctx),
                                                          void (*on_done)(void *stream_ctx),
                                                          void *stream_ctx);

/* ── Resource management ────────────────────────────────────────────────── */

/**
 * Release a handle. Safe to call with 0 or an unknown handle (no-op).
 *
 * @param handle Handle from aimux_*_new.
 */
void aimux_drop_handle(uint64_t handle);

/**
 * Free a string written by any aimux function (`*out_json`, error getters).
 * (Handles are released with aimux_drop_handle, errors with
 * aimux_error_free, not here.)
 *
 * @param ptr Pointer from an aimux_* function (NULL is safe).
 */
void aimux_free_string(char *ptr);

/* ── Embedding ───────────────────────────────────────────────────────────── */
/* [C ABI] Multimodal constructors below cannot fail as an AiMuxError (they only
   store the config); [AiMuxError] the `*_generate`-style calls write the
   modality's Result JSON to *out_json. All strings must be freed with
   aimux_free_string. */

aimux_error_t *aimux_openai_embedding_new(const char *api_key, const char *model_id, uint64_t *out_handle);
aimux_error_t *aimux_openai_embedding_new_with_base(const char *api_key, const char *model_id, const char *base_url, uint64_t *out_handle);
aimux_error_t *aimux_cohere_embedding_new(const char *api_key, const char *model_id, uint64_t *out_handle);
aimux_error_t *aimux_cohere_embedding_new_with_base(const char *api_key, const char *model_id, const char *base_url, uint64_t *out_handle);
aimux_error_t *aimux_google_embedding_new(const char *api_key, const char *model_id, uint64_t *out_handle);
aimux_error_t *aimux_google_embedding_new_with_base(const char *api_key, const char *model_id, const char *base_url, uint64_t *out_handle);
/* values_json: JSON array of strings; opts_json: EmbeddingCallOptions or NULL. */
aimux_error_t *aimux_embed(uint64_t handle, const char *values_json, const char *opts_json, char **out_json);

/* ── Speech (TTS) ────────────────────────────────────────────────────────── */

aimux_error_t *aimux_openai_speech_new(const char *api_key, const char *model_id, uint64_t *out_handle);
aimux_error_t *aimux_openai_speech_new_with_base(const char *api_key, const char *model_id, const char *base_url, uint64_t *out_handle);
/* opts_json is REQUIRED (it carries the input — text / prompt / documents /
   query — not just options); NULL or empty is a C ABI failure. */
aimux_error_t *aimux_speech_generate(uint64_t handle, const char *opts_json, char **out_json);

/* ── Image ──────────────────────────────────────────────────────────────── */

aimux_error_t *aimux_openai_image_new(const char *api_key, const char *model_id, uint64_t *out_handle);
aimux_error_t *aimux_openai_image_new_with_base(const char *api_key, const char *model_id, const char *base_url, uint64_t *out_handle);
aimux_error_t *aimux_google_image_new(const char *api_key, const char *model_id, uint64_t *out_handle);
aimux_error_t *aimux_google_image_new_with_base(const char *api_key, const char *model_id, const char *base_url, uint64_t *out_handle);
/* opts_json is REQUIRED (it carries the input — text / prompt / documents /
   query — not just options); NULL or empty is a C ABI failure. */
aimux_error_t *aimux_image_generate(uint64_t handle, const char *opts_json, char **out_json);

/* ── Transcription (STT, non-streaming) ──────────────────────────────────── */

aimux_error_t *aimux_openai_transcription_new(const char *api_key, const char *model_id, uint64_t *out_handle);
aimux_error_t *aimux_openai_transcription_new_with_base(const char *api_key, const char *model_id, const char *base_url, uint64_t *out_handle);
/* opts_json is currently IGNORED (reserved for future options). */
aimux_error_t *aimux_transcription_generate(uint64_t handle, const char *audio_base64, const char *media_type, const char *opts_json, char **out_json);

/* ── Transcription streaming sessions (RFC-0028) ─────────────────────────── */

/**
 * Pull states of aimux_transcription_next_part, written to *out_state (an
 * int32_t — the ABI does not depend on the C compiler's enum width) when the
 * call returns NULL.
 */
typedef enum aimux_transcription_next_part_state {
    AIMUX_TRANSCRIPTION_NEXT_PART_PART = 1,    /* *out_part holds a part */
    AIMUX_TRANSCRIPTION_NEXT_PART_ENDED = 2,   /* stream ended normally; *out_part NULL */
    AIMUX_TRANSCRIPTION_NEXT_PART_TIMEOUT = 3  /* no part in time, session live; *out_part NULL */
} aimux_transcription_next_part_state_t;

/* [AiMuxError] Start a streaming transcription session. The driver task spawns
   immediately; push audio with aimux_transcription_push_audio, then pull
   parts with aimux_transcription_next_part. model_handle must support
   streaming (models without do_stream fail on the first next_part).
   abort_handle: 0 = no cancellation, or an aimux_abort_signal_new handle.
   opts_json (all optional): { "input_audio_format": {"format_type","rate"},
   "provider_options", "headers", "include_raw_chunks", "timeout":
   {"total_ms","first_chunk_ms","chunk_ms"} }; NULL/empty/"null" = defaults
   (wrong shape → AiMuxError AIMUX_E_INVALID_ARGUMENT). Writes the session handle. */
aimux_error_t *aimux_transcription_session_new(uint64_t model_handle, uint64_t abort_handle,
                                                   const char *opts_json, uint64_t *out_handle);

/* [AiMuxError] Push one binary audio chunk. BLOCKING: waits while the internal
   channel is full (backpressure propagation). data may be NULL only when
   len == 0 (no-op). Pushing after aimux_transcription_input_done or after
   the session ended is an AiMuxError. Not callable from within an aimux
   callback (re-entrancy guard — applies to next_part too). */
aimux_error_t *aimux_transcription_push_audio(uint64_t session, const uint8_t *data,
                                                  size_t len);

/* [C ABI] Signal end-of-audio (idempotent). Fails only for a dead handle. */
aimux_error_t *aimux_transcription_input_done(uint64_t session);

/* [AiMuxError] Pull the next part. timeout_ms: >0 = wait at most; 0 = immediate
   poll; <0 = wait indefinitely. Both out-params are required. On NULL
   return, *out_state says what happened: PART (*out_part holds JSON
   TranscriptionStreamPart; free with aimux_free_string), ENDED or TIMEOUT
   (*out_part NULL). A non-NULL return is a failure (abort / API error →
   AiMuxError view; dead handle → C ABI failure); *out_part is NULL and *out_state is
   unspecified. */
aimux_error_t *aimux_transcription_next_part(uint64_t session, int64_t timeout_ms,
                                                 char **out_part,
                                                 int32_t *out_state);

/* Terminate and release the session (aborts the driver; safe with 0). */
void aimux_transcription_session_drop(uint64_t session);

/* ── Files ──────────────────────────────────────────────────────────────── */

aimux_error_t *aimux_openai_files_new(const char *api_key, uint64_t *out_handle);
aimux_error_t *aimux_openai_files_new_with_base(const char *api_key, const char *base_url, uint64_t *out_handle);
/* opts_json is currently IGNORED (reserved for future options). */
aimux_error_t *aimux_file_upload(uint64_t handle, const char *data_base64, const char *media_type, const char *opts_json, char **out_json);

/* ── Reranking ───────────────────────────────────────────────────────────── */

aimux_error_t *aimux_cohere_reranking_new(const char *api_key, const char *model_id, uint64_t *out_handle);
aimux_error_t *aimux_cohere_reranking_new_with_base(const char *api_key, const char *model_id, const char *base_url, uint64_t *out_handle);
/* opts_json is REQUIRED (it carries the input — text / prompt / documents /
   query — not just options); NULL or empty is a C ABI failure. */
aimux_error_t *aimux_rerank(uint64_t handle, const char *opts_json, char **out_json);

/* ── Video ───────────────────────────────────────────────────────────────── */

aimux_error_t *aimux_google_video_new(const char *api_key, const char *model_id, uint64_t *out_handle);
aimux_error_t *aimux_google_video_new_with_base(const char *api_key, const char *model_id, const char *base_url, uint64_t *out_handle);
/* opts_json is REQUIRED (it carries the input — text / prompt / documents /
   query — not just options); NULL or empty is a C ABI failure. */
aimux_error_t *aimux_video_generate(uint64_t handle, const char *opts_json, char **out_json);

/* ── Search ──────────────────────────────────────────────────────────────── */

/* model_id is accepted for API symmetry but ignored (Tavily uses a fixed
   endpoint). */
aimux_error_t *aimux_tavily_search_new(const char *api_key, const char *model_id, uint64_t *out_handle);
aimux_error_t *aimux_tavily_search_new_with_base(const char *api_key, const char *model_id, const char *base_url, uint64_t *out_handle);
/* opts_json is REQUIRED (it carries the input — text / prompt / documents /
   query — not just options); NULL or empty is a C ABI failure. */
aimux_error_t *aimux_search(uint64_t handle, const char *opts_json, char **out_json);

/* Codex (RFC-0018) */

/* [AiMuxError] Refresh a Codex subscription access token (stateless OAuth
   helper). Writes JSON {"access_token","refresh_token","expires_in_secs"};
   caller frees. Caller owns token persistence and the 401 -> refresh ->
   retry orchestration. */
aimux_error_t *aimux_codex_refresh(const char *refresh_token, const char *client_id,
                                       char **out_json);

/* Logging (RFC-0014) */

/* [C ABI] Initialize the global logger (idempotent, thread-safe, no-op if the
   host already registered its own subscriber). level: "off"|"error"|"warn"|
   "info"|"debug"|"trace" (NULL = default "warn"; non-UTF-8 is a C ABI
   failure); AIMUX_LOG and AIMUX_LOG_LEVEL env vars take precedence. Logs go
   to stderr. */
aimux_error_t *aimux_init_logging(const char *level);

/* Session grouping (RFC-0024) */

/* Register the global session store (replaces any previous one). Until
   called, calls are not grouped and the session query functions return
   empty results. Infallible. */
void aimux_session_store_init(void);

/* Enable/disable the global session inferer (opt-in, off by default).
   enabled nonzero = on; explicit session_id always wins. Infallible. */
void aimux_session_infer_init(int32_t enabled);

/* [C ABI] Query: all calls of a session, ordered by step. Writes a JSON
   SessionCall[] (empty if unknown / no store); caller frees with
   aimux_free_string. */
aimux_error_t *aimux_session_calls(const char *session_id, char **out_json);

/* [C ABI] Query: all known sessions. Writes a JSON SessionView[]; caller
   frees with aimux_free_string. */
aimux_error_t *aimux_list_sessions(char **out_json);

/* Cache probing (RFC-0015) */

/* [C ABI] Wrap a model handle in a probe layer. The new handle works with
   aimux_generate_text / aimux_stream_text (probed) and the aimux_trace_*
   queries; release with aimux_drop_handle. */
aimux_error_t *aimux_trace_new(uint64_t handle, uint64_t *out_handle);

/* [C ABI] Same, with the built-in rules auditor attached. strict nonzero =
   strict mode; zero = shared (safe default). */
aimux_error_t *aimux_trace_new_audited(uint64_t handle, int32_t strict, uint64_t *out_handle);

/* [AiMuxError] Query: aggregated probe statistics, filtered by filter_json
   (serialized TraceFilter; "{}" = all; NULL is a C ABI failure). Writes
   JSON TraceStats[]; caller frees. */
aimux_error_t *aimux_trace_aggregate(uint64_t handle, const char *filter_json, char **out_json);

/* [AiMuxError] Query: one session's chain view. Writes JSON SessionChainView;
   caller frees. An unknown session_id is a lookup miss on a caller-supplied
   string key and is AiMuxError AIMUX_E_INVALID_ARGUMENT ("unknown session"), not
   a C ABI handle failure. */
aimux_error_t *aimux_trace_session_chain(uint64_t handle, const char *session_id, char **out_json);

/* [C ABI] Query: one session's per-step trajectory. Writes a JSON array
   (empty for unknown sessions); caller frees. */
aimux_error_t *aimux_trace_session_trajectory(uint64_t handle, const char *session_id, char **out_json);

/* [C ABI] Export all probe records as JSONL (one TraceRecord per line). Writes
   a string with embedded newlines; caller frees. */
aimux_error_t *aimux_trace_export_jsonl(uint64_t handle, char **out_jsonl);

/* [C ABI] Clear all probe records of a trace handle. Fails only for a dead
   handle. */
aimux_error_t *aimux_trace_clear(uint64_t handle);

/* ── Recording + mock replay (RFC-0023) ──────────────────────────────── */

/* [RecordingError] Start recording: complete Recording JSONL is written to
   {dir}/recordings.jsonl (dir auto-created). Recording is opt-in; calling
   again replaces the recorder. A NULL or non-UTF-8 dir is a C ABI
   failure; the recorder failing to create the directory, open the file or
   start its writer carries the recording view (AIMUX_E_RECORDING_INIT /
   _OPEN_FILE / _SPAWN). */
aimux_error_t *aimux_init_recording(const char *dir);

/* [AiMuxError] Start in-memory bounded recording (RingRecorder, FIFO eviction,
   dropped count queryable). cap == 0 is AiMuxError AIMUX_E_INVALID_ARGUMENT. */
aimux_error_t *aimux_init_recording_ring(uint64_t cap);

/* No-argument variant: start in-memory bounded recording with the library
   default ring capacity (2048 entries). Ordinary callers should prefer this
   entry point; pass an explicit cap via aimux_init_recording_ring only when a
   different size is required. Infallible. */
void aimux_init_recording_ring_default(void);

/* Stop recording: the global recorder becomes None. Infallible. */
void aimux_recording_stop(void);

/* Flush the global recorder (blocks until JSONL is on disk; no-op for the
   ring recorder). Write failures are not reported here — see
   aimux_recording_try_flush. Infallible. */
void aimux_recording_flush(void);

/* [RecordingError] Flush the global recorder and report write failures. NULL
   when the JSONL is confirmed on disk (also when recording was never
   initialized); otherwise an error with the recording view — read it
   with aimux_error_code() / aimux_error_message() (see
   aimux-error.h). Reachable codes:
     AIMUX_E_RECORDING_WRITE (6) — a prior write failed (sticky, e.g. ENOSPC)
     AIMUX_E_RECORDING_WRITER_GONE (4) — writer unavailable
     AIMUX_E_RECORDING_FLUSH_TIMEOUT (5) — no writer ack within 30s */
aimux_error_t *aimux_recording_try_flush(void);

/* [AiMuxError] Register external OpenAI-compatible providers from a JSON config
   string (RFC-0020). config_json is { "providers": [ { "name", "base_url",
   ... } ] }. Entries override same-named built-ins or add new ones. Malformed
   JSON text is a C ABI failure; a well-formed document the registry
   rejects (bad schema, unknown protocol) is AiMuxError AIMUX_E_INVALID_ARGUMENT. */
aimux_error_t *aimux_register_providers(const char *config_json);

/* [AiMuxError] Set the global proxy configuration (M6, RFC-0016). Must be called
   before the first generate_text / stream_text call; a no-op if the shared
   HTTP client is already initialised. config_json is a serialized
   ProxyConfig: { "http_url", "https_url", "all_url", "no_proxy" } (all
   optional). Malformed text is a C ABI failure; wrong shape is AiMuxError
   AIMUX_E_INVALID_ARGUMENT. */
aimux_error_t *aimux_init_proxy(const char *config_json);

/* [AiMuxError] Create a mock replay model from recorded JSONL (one Recording per
   line). The handle works with aimux_generate_text / aimux_stream_text (no
   real API sent); release with aimux_drop_handle. Malformed line → C ABI
   failure; wrong shape or no recordings → AiMuxError AIMUX_E_INVALID_ARGUMENT. */
aimux_error_t *aimux_mock_replay_new(const char *recordings_jsonl, uint64_t *out_handle);

/* ── Composite models (RFC-0021 / RFC-0022) ─────────────────────────── */

/* [AiMuxError] Create a RouterModel (RFC-0021) over the given child-model handles.
   The new handle is itself a model handle (works with aimux_generate_text /
   aimux_stream_text); release with aimux_drop_handle.

   handles: array of `len` live model handles (e.g. from aimux_openai_new).
   Any dead handle in the array is a C ABI failure — nothing is silently
   dropped; len == 0 (no children) is a C ABI failure too; NULL with len > 0
   is a C ABI failure. config_json selects the router + fallback policy:
   { "router": "rule"|"weighted", "weights": [..], "fallback": "on_error"|"none",
     "provider_name": "router", "model_id": "router" } — all optional; defaults
   are rule / on_error / "router" / "router" (wrong shape → AiMuxError
   AIMUX_E_INVALID_ARGUMENT). */
aimux_error_t *aimux_router_new(const uint64_t *handles, size_t len,
                                    const char *config_json, uint64_t *out_handle);

/* [AiMuxError] Create a MoaModel (RFC-0022) over reference handles + one aggregator
   handle. The new handle is a model handle (works with aimux_generate_text /
   aimux_stream_text); release with aimux_drop_handle.

   reference_handles: array of `ref_len` live model handles (NULL with
   ref_len == 0 = no references — MoaModel then runs just the aggregator; NULL
   with ref_len > 0 is a C ABI failure). aggregator: a single live handle.
   Any dead reference or aggregator handle is a C ABI failure — nothing is
   silently dropped. config_json is a serialized MoaConfig (all fields
   optional): { "provider_name", "model_id", "aggregator_instructions",
   "strip_reference_tools", "fail_mode": "best_effort"|"fail_fast" } (wrong shape →
   AiMuxError AIMUX_E_INVALID_ARGUMENT). */
aimux_error_t *aimux_moa_new(const uint64_t *reference_handles, size_t ref_len,
                                 uint64_t aggregator, const char *config_json,
                                 uint64_t *out_handle);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* AIMUX_FFI_H */
