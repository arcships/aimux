/**
 * aimux-ffi.h — C ABI for aimux multi-language bindings.
 *
 * This is the C ABI boundary layer. Only used by C ABI bindings
 * (Swift / Kotlin / C / C++). Native bindings (Python / Node / Flutter)
 * bypass this layer and use aimux-providers directly.
 *
 * ## Memory ownership
 *
 * Fallible calls take a trailing `AimuxError *err` (may be NULL). Success
 * returns the result (`uint64_t` handle or owned `char*` JSON). Failure
 * returns `0` (handles / stream status) or `NULL` (payload pointers). When
 * `err` is non-NULL on failure, the callee fills `*err`; free `err->message`
 * with `aimux_free_string`. The return value is the only success/failure
 * signal.
 * Result strings are freed with `aimux_free_string` (`NULL` is safe).
 * `aimux_stream_text` callbacks receive `const char*`
 * pointers that are valid **only for the duration of the callback**. The
 * callback must copy the data synchronously.
 *
 * ## Concurrency
 *
 * All functions are synchronous (block until the operation completes).
 * Callbacks execute on the same thread; do not re-enter the FFI layer
 * from a callback (would deadlock the tokio runtime).
 * `opts_json.timeout` (`{"total_ms","first_chunk_ms","chunk_ms"}`,
 * milliseconds; the latter two streaming-only) bounds a call from the callee
 * side; `aimux_abort_signal_*` plus the `*_with_abort` entry points cancel a
 * running call from another thread.
 *
 * ## Wire format
 *
 * - Errors: on failure, optional `AimuxError *err` is filled (see aimux-error.h);
 *   free `err->message` with aimux_free_string. No JSON error envelope.
 * - Internal panics abort the process (the workspace builds with panic=abort).

 * - `prompt_json`: bare prompt value (`"text"` or `[{...}]`), or a
 *   single-key wrapper `{"prompt": <value>}` (any extra key disables
 *   unwrapping and the whole object is parsed as the prompt)
 * - `opts_json` for `aimux_generate_text`/`aimux_stream_text`: serialized
 *   GenerateTextOptions (empty/null for defaults). Multimodal calls
 *   (`aimux_speech_generate`, `aimux_image_generate`, `aimux_rerank`,
 *   `aimux_video_generate`, `aimux_search`) REQUIRE a valid JSON object —
 *   NULL or empty fails (fills `*err` when non-NULL).
 * - Results: serialized JSON of GenerateTextResult on success
 * - Stream parts: serialized JSON of StreamPart
 * - Constructors: `uint64_t` handle on success (`0` = failure)
 */

#ifndef AIMUX_FFI_H
#define AIMUX_FFI_H

#include <stdint.h>

#include "aimux-error.h"

#ifdef __cplusplus
extern "C" {
#endif

/* ── Provider constructors ──────────────────────────────────────────────── */

/**
 * Create an OpenAI model instance.
 *
 * @param api_key  NUL-terminated API key string.
 * @param model_id NUL-terminated model ID (e.g. "gpt-4o").
 * @return Handle > 0 on success, or 0 with details in `*err` (if non-NULL) on failure.
 */
uint64_t aimux_openai_new(const char *api_key, const char *model_id, AimuxError *err);

/**
 * Create an OpenAI model instance with a custom base URL.
 *
 * @param api_key  NUL-terminated API key string.
 * @param model_id NUL-terminated model ID (e.g. "gpt-4o").
 * @param base_url NUL-terminated base URL (NULL or empty uses the provider default).
 * @return Handle > 0 on success, or 0 with details in `*err` (if non-NULL) on failure.
 */
uint64_t aimux_openai_new_with_base(const char *api_key,
                                 const char *model_id,
                                 const char *base_url, AimuxError *err);

/**
 * Create an Anthropic model instance.
 *
 * @param api_key  NUL-terminated API key string.
 * @param model_id NUL-terminated model ID (e.g. "claude-3-5-sonnet-20241022").
 * @return Handle > 0 on success, or 0 with details in `*err` (if non-NULL) on failure.
 */
uint64_t aimux_anthropic_new(const char *api_key, const char *model_id, AimuxError *err);

/**
 * Create an Anthropic model instance with a custom base URL.
 *
 * @param api_key  NUL-terminated API key string.
 * @param model_id NUL-terminated model ID (e.g. "claude-3-5-sonnet-20241022").
 * @param base_url NUL-terminated base URL (NULL or empty uses the provider default).
 * @return Handle > 0 on success, or 0 with details in `*err` (if non-NULL) on failure.
 */
uint64_t aimux_anthropic_new_with_base(const char *api_key,
                                    const char *model_id,
                                    const char *base_url, AimuxError *err);

/**
 * Create a Cohere model instance (API key + model ID).
 * Returns handle > 0 on success, or 0 with details in `*err` on failure.
 * `_with_base` adds `base_url` (NULL or empty uses the provider default).
 */
uint64_t aimux_cohere_new(const char *api_key, const char *model_id, AimuxError *err);
uint64_t aimux_cohere_new_with_base(const char *api_key,
                                 const char *model_id,
                                 const char *base_url, AimuxError *err);

/**
 * Create a Mistral model instance (API key + model ID).
 * Returns handle > 0 on success, or 0 with details in `*err` on failure.
 * `_with_base` adds `base_url` (NULL or empty uses the provider default).
 */
uint64_t aimux_mistral_new(const char *api_key, const char *model_id, AimuxError *err);
uint64_t aimux_mistral_new_with_base(const char *api_key,
                                  const char *model_id,
                                  const char *base_url, AimuxError *err);

/**
 * Create an xAI model instance (API key + model ID).
 * Returns handle > 0 on success, or 0 with details in `*err` on failure.
 * `_with_base` adds `base_url` (NULL or empty uses the provider default).
 */
uint64_t aimux_xai_new(const char *api_key, const char *model_id, AimuxError *err);
uint64_t aimux_xai_new_with_base(const char *api_key,
                              const char *model_id,
                              const char *base_url, AimuxError *err);

/**
 * Create a Bedrock model instance (AWS SigV4 credentials).
 * Returns handle > 0 on success, or 0 with details in `*err` on failure.
 * `_with_base` adds `base_url` (NULL or empty uses the provider default).
 */
uint64_t aimux_bedrock_new(const char *access_key_id,
                        const char *secret_access_key,
                        const char *region,
                        const char *model_id, AimuxError *err);
uint64_t aimux_bedrock_new_with_base(const char *access_key_id,
                                  const char *secret_access_key,
                                  const char *region,
                                  const char *model_id,
                                  const char *base_url, AimuxError *err);

/**
 * Create a Vertex AI model instance (GCP bearer token).
 * Returns handle > 0 on success, or 0 with details in `*err` on failure.
 * `_with_base` adds `base_url` (NULL or empty uses the provider default).
 */
uint64_t aimux_vertex_new(const char *access_token,
                       const char *project,
                       const char *location,
                       const char *model_id, AimuxError *err);
uint64_t aimux_vertex_new_with_base(const char *access_token,
                                 const char *project,
                                 const char *location,
                                 const char *model_id,
                                 const char *base_url, AimuxError *err);

/**
 * Create an Anthropic-on-AWS model instance (API key + region).
 * Returns handle > 0 on success, or 0 with details in `*err` on failure.
 * `_with_base` adds `base_url` (NULL or empty uses the provider default).
 */
uint64_t aimux_anthropic_aws_new(const char *api_key, const char *region,
                              const char *model_id, AimuxError *err);
uint64_t aimux_anthropic_aws_new_with_base(const char *api_key, const char *region,
                                        const char *model_id, const char *base_url, AimuxError *err);

/**
 * Create an Azure OpenAI model instance (API key + resource name; deployment
 * passed as model_id; api_version NULL or empty uses the provider default).
 * Returns handle > 0 on success, or 0 with details in `*err` on failure.
 *
 * `_with_base` takes an explicit `base_url` IN PLACE OF `resource_name`.
 * Unlike other `_with_base` variants, `base_url` here is REQUIRED — NULL
 * returns 0 with AIMUX_E_INVALID_ARGUMENT in `*err`.
 */
uint64_t aimux_azure_new(const char *api_key, const char *resource_name,
                      const char *deployment, const char *api_version, AimuxError *err);
uint64_t aimux_azure_new_with_base(const char *api_key, const char *base_url,
                                const char *deployment, const char *api_version, AimuxError *err);

/**
 * Create a model from the provider registry by name (RFC-0017 phase 4).
 *
 * @param name        NUL-terminated registry provider name (e.g. "deepseek", "groq").
 * @param api_key     NUL-terminated API key; NULL (or non-UTF-8) reads the
 *                    provider's env var from the registry entry.
 * @param model_id    NUL-terminated model ID.
 * @param config_json Optional JSON object of ProviderOptions
 *                    ({"base_url": "...", "headers": {...}, "max_retries": 0,
 *                     "body_overrides": {...}});
 *                    NULL / empty / "null" for defaults.
 * @return Handle > 0 on success, or 0 with details in `*err` (unknown
 *         provider, bad config, missing env key, or invalid model id).
 */
uint64_t aimux_provider_new(const char *name, const char *api_key,
                         const char *model_id, const char *config_json, AimuxError *err);

/**
 * Convenience: create a model by provider name, reading the API key from the
 * provider's env var. Handle > 0 on success, or 0 with details in `*err`
 * (if non-NULL).
 */
uint64_t aimux_provider_from_env(const char *name, const char *model_id, AimuxError *err);

/**
 * Provider handles (RFC-0027): create a provider handle for a registry-backed
 * provider. Unlike aimux_provider_new (which binds to a single model_id),
 * this returns a provider handle supporting aimux_provider_list_models and
 * aimux_provider_model. Handle > 0 on success, or 0 with details in `*err`.
 */
uint64_t aimux_provider_handle_new(const char *name, const char *api_key,
                                const char *config_json, AimuxError *err);

/**
 * List models on a provider handle (RFC-0027 runtime discovery).
 * Returns a JSON array of sparse RuntimeModel (id / owned_by / created) —
 * no community enrichment. To supplement with model specs, call
 * aimux_get_model_specs separately and merge in the host.
 * Returns NULL on failure (fills `*err`).
 */
char *aimux_provider_list_models(uint64_t handle, AimuxError *err);

/**
 * Build a language model from a provider handle + model_id.
 * Handle > 0 on success, or 0 with details in `*err`.
 */
uint64_t aimux_provider_model(uint64_t handle, const char *model_id, AimuxError *err);

/**
 * Fetch the community model catalogue (anya2a). Returns a JSON-serialized
 * Catalogue (provider → model_id → ModelSpec), or NULL with details in `*err`.
 * source_url may be NULL for the default endpoint. Thin fetch — no caching.
 */
char *aimux_get_model_specs(const char *source_url, AimuxError *err);

/* ── Generation ─────────────────────────────────────────────────────────── */

/**
 * Non-streaming text generation.
 *
 * @param handle      Language-model handle from aimux_*_new
 *                    (a handle of another modality yields "invalid handle").
 * @param prompt_json JSON prompt (see wire format above).
 * @param opts_json   JSON options (NULL or empty for defaults).
 * @return JSON result string (caller MUST free with aimux_free_string).
 *         On error: returns NULL and fills `*err` when non-NULL.
 *         Malformed prompt_json/opts_json fails the same way before any network call.
 */
char *aimux_generate_text(uint64_t handle,
                          const char *prompt_json,
                          const char *opts_json, AimuxError *err);

/**
 * Streaming text generation with push callbacks (blocks until stream ends).
 *
 * @param handle      Model handle from aimux_*_new.
 * @param prompt_json JSON prompt (see wire format above).
 * @param opts_json   JSON options (NULL or empty for defaults).
 * @param on_part     Called for each StreamPart (JSON string, valid during call only).
 * @param on_done     Called once when the stream ends normally.
 * @param stream_ctx  Opaque pointer passed through to on_part / on_done.
 * @param err   On failure, filled when non-NULL (free err->message with aimux_free_string).
 * @return non-zero on success; 0 on failure (details in `*err`).
 */
int32_t aimux_stream_text(uint64_t handle,
                          const char *prompt_json,
                          const char *opts_json,
                          void (*on_part)(const char *json, void *stream_ctx),
                          void (*on_done)(void *stream_ctx),
                          void *stream_ctx,
                          AimuxError *err);

/**
 * Create a per-call abort signal.
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
 * Streaming text generation with per-call cancellation.
 *
 * This function blocks until the stream ends. Another thread can call
 * aimux_abort_signal_abort while this function runs. Cancellation fails the
 * call with an Aborted details in `*err` and does not call on_done.
 *
 * @param handle       Model handle from aimux_*_new.
 * @param abort_handle Handle from aimux_abort_signal_new.
 * @param prompt_json  JSON prompt.
 * @param opts_json    JSON options. NULL or empty uses defaults.
 * @param on_part      Called for each StreamPart.
 * @param on_done      Called once after normal completion.
 * @param stream_ctx   Opaque pointer passed through to callbacks.
 * @param err    On failure, filled when non-NULL (free err->message with aimux_free_string).
 * @return non-zero on success; 0 on failure (details in `*err`).
 */
int32_t aimux_stream_text_with_abort(uint64_t handle, uint64_t abort_handle,
                                     const char *prompt_json,
                                     const char *opts_json,
                                     void (*on_part)(const char *json, void *stream_ctx),
                                     void (*on_done)(void *stream_ctx),
                                     void *stream_ctx,
                                     AimuxError *err);

/* ── OpenAI-compatible output (RFC-0026) ───────────────────────────────── */

/**
 * Non-streaming text generation with OpenAI Chat Completions output.
 *
 * Same as aimux_generate_text, but returns a serialized ChatCompletion
 * (OpenAI "chat.completion" object). Works with any provider.
 *
 * @return JSON ChatCompletion string (caller MUST free with aimux_free_string).
 */
char *aimux_generate_text_as_openai(uint64_t handle,
                                    const char *prompt_json,
                                    const char *opts_json, AimuxError *err);

/**
 * Streaming text generation with OpenAI Chat Completions output.
 *
 * Same as aimux_stream_text, but each on_part receives a serialized
 * ChatCompletionChunk (OpenAI "chat.completion.chunk" object).
 * Works with any provider.
 *
 * @param opts_json May carry providerOptions.openai.stream_options with
 *                  include_usage (bool, default true) and
 *                  include_reasoning (bool, default true).
 */
int32_t aimux_stream_text_as_openai(uint64_t handle,
                                    const char *prompt_json,
                                    const char *opts_json,
                                    void (*on_part)(const char *json, void *stream_ctx),
                                    void (*on_done)(void *stream_ctx),
                                    void *stream_ctx,
                                    AimuxError *err);

/**
 * Cancelable streaming OpenAI-compatible output (see aimux_stream_text_with_abort).
 */
int32_t aimux_stream_text_as_openai_with_abort(uint64_t handle, uint64_t abort_handle,
                                               const char *prompt_json,
                                               const char *opts_json,
                                               void (*on_part)(const char *json, void *stream_ctx),
                                               void (*on_done)(void *stream_ctx),
                                               void *stream_ctx,
                                               AimuxError *err);

/* ── Resource management ────────────────────────────────────────────────── */

/**
 * Release a model handle. Safe to call with 0 (no-op).
 *
 * @param handle Model handle from aimux_*_new.
 */
void aimux_drop_handle(uint64_t handle);

/**
 * Free a string previously returned by aimux_generate_text, any aimux_*_new
 * constructor, or other functions returning char*.
 *
 * @param ptr Pointer from an aimux_* function (NULL is safe).
 */
void aimux_free_string(char *ptr);

/* ── Embedding ───────────────────────────────────────────────────────────── */
/* Constructors below return uint64_t handle (0 = fail, details in *err);
   `*_generate`-style calls return the modality's Result JSON, or NULL on
   failure (fills *err). All returned strings must be freed with
   aimux_free_string. */

uint64_t aimux_openai_embedding_new(const char *api_key, const char *model_id, AimuxError *err);
uint64_t aimux_openai_embedding_new_with_base(const char *api_key, const char *model_id, const char *base_url, AimuxError *err);
uint64_t aimux_cohere_embedding_new(const char *api_key, const char *model_id, AimuxError *err);
uint64_t aimux_cohere_embedding_new_with_base(const char *api_key, const char *model_id, const char *base_url, AimuxError *err);
uint64_t aimux_google_embedding_new(const char *api_key, const char *model_id, AimuxError *err);
uint64_t aimux_google_embedding_new_with_base(const char *api_key, const char *model_id, const char *base_url, AimuxError *err);
char *aimux_embed(uint64_t handle, const char *values_json, const char *opts_json, AimuxError *err);

/* ── Speech (TTS) ────────────────────────────────────────────────────────── */

uint64_t aimux_openai_speech_new(const char *api_key, const char *model_id, AimuxError *err);
uint64_t aimux_openai_speech_new_with_base(const char *api_key, const char *model_id, const char *base_url, AimuxError *err);
char *aimux_speech_generate(uint64_t handle, const char *opts_json, AimuxError *err);

/* ── Image ──────────────────────────────────────────────────────────────── */

uint64_t aimux_openai_image_new(const char *api_key, const char *model_id, AimuxError *err);
uint64_t aimux_openai_image_new_with_base(const char *api_key, const char *model_id, const char *base_url, AimuxError *err);
uint64_t aimux_google_image_new(const char *api_key, const char *model_id, AimuxError *err);
uint64_t aimux_google_image_new_with_base(const char *api_key, const char *model_id, const char *base_url, AimuxError *err);
char *aimux_image_generate(uint64_t handle, const char *opts_json, AimuxError *err);

/* ── Transcription (STT, non-streaming) ──────────────────────────────────── */

uint64_t aimux_openai_transcription_new(const char *api_key, const char *model_id, AimuxError *err);
uint64_t aimux_openai_transcription_new_with_base(const char *api_key, const char *model_id, const char *base_url, AimuxError *err);
/* opts_json is currently IGNORED (reserved for future options). */
char *aimux_transcription_generate(uint64_t handle, const char *audio_base64, const char *media_type, const char *opts_json, AimuxError *err);

/* ── Files ──────────────────────────────────────────────────────────────── */

uint64_t aimux_openai_files_new(const char *api_key, AimuxError *err);
uint64_t aimux_openai_files_new_with_base(const char *api_key, const char *base_url, AimuxError *err);
/* opts_json is currently IGNORED (reserved for future options). */
char *aimux_file_upload(uint64_t handle, const char *data_base64, const char *media_type, const char *opts_json, AimuxError *err);

/* ── Reranking ───────────────────────────────────────────────────────────── */

uint64_t aimux_cohere_reranking_new(const char *api_key, const char *model_id, AimuxError *err);
uint64_t aimux_cohere_reranking_new_with_base(const char *api_key, const char *model_id, const char *base_url, AimuxError *err);
char *aimux_rerank(uint64_t handle, const char *opts_json, AimuxError *err);

/* ── Video ───────────────────────────────────────────────────────────────── */

uint64_t aimux_google_video_new(const char *api_key, const char *model_id, AimuxError *err);
uint64_t aimux_google_video_new_with_base(const char *api_key, const char *model_id, const char *base_url, AimuxError *err);
char *aimux_video_generate(uint64_t handle, const char *opts_json, AimuxError *err);

/* ── Search ──────────────────────────────────────────────────────────────── */

/* model_id is accepted for API symmetry but ignored (Tavily uses a fixed
   endpoint). */
uint64_t aimux_tavily_search_new(const char *api_key, const char *model_id, AimuxError *err);
uint64_t aimux_tavily_search_new_with_base(const char *api_key, const char *model_id, const char *base_url, AimuxError *err);
char *aimux_search(uint64_t handle, const char *opts_json, AimuxError *err);

/* Codex (RFC-0018) */

/* Refresh a Codex subscription access token (stateless OAuth helper).
   Returns JSON {"access_token","refresh_token","expires_in_secs"}, or NULL
   on failure (fills *err); caller frees. Caller owns token persistence
   and the 401 -> refresh -> retry orchestration. */
char *aimux_codex_refresh(const char *refresh_token, const char *client_id, AimuxError *err);

/* Logging (RFC-0014) */

/* Initialize the global logger (idempotent, thread-safe, no-op if the host
   already registered its own subscriber). level: "off"|"error"|"warn"|
   "info"|"debug"|"trace" (NULL = default "warn"); AIMUX_LOG and
   AIMUX_LOG_LEVEL env vars take precedence. Logs go to stderr.
   Returns 0. */
int aimux_init_logging(const char *level);

/* Session grouping (RFC-0024) */

/* Register the global session store (replaces any previous one). Until
   called, calls are not grouped and the session query functions return
   empty results. Returns 0. */
int aimux_session_store_init(void);

/* Enable/disable the global session inferer (opt-in, off by default).
   enabled nonzero = on; explicit session_id always wins. Returns 0. */
int aimux_session_infer_init(int enabled);

/* Query: all calls of a session, ordered by step. Returns a JSON
   SessionCall[] (empty if unknown / no store), or NULL on failure; caller frees
   with aimux_free_string. */
char *aimux_session_calls(const char *session_id, AimuxError *err);

/* Query: all known sessions. Returns a JSON SessionView[] or
   NULL on failure (fills *err); caller frees with aimux_free_string. */
char *aimux_list_sessions(AimuxError *err);

/* Cache probing (RFC-0015) */

/* Wrap a model handle in a probe layer. The returned handle works with
   aimux_generate_text / aimux_stream_text (probed) and the aimux_trace_*
   queries. Returns handle > 0 or 0 on failure (fills *err). */
uint64_t aimux_trace_new(uint64_t handle, AimuxError *err);

/* Same, with the built-in rules auditor attached. strict nonzero = strict
   mode; zero = shared (safe default). */
uint64_t aimux_trace_new_audited(uint64_t handle, int strict, AimuxError *err);

/* Query: aggregated probe statistics, filtered by filter_json (serialized
   TraceFilter; NULL = all). Returns JSON TraceStats[]; caller frees. */
char *aimux_trace_aggregate(uint64_t handle, const char *filter_json, AimuxError *err);

/* Query: one session's chain view. Returns JSON SessionChainView or
   NULL on failure (fills *err, e.g. unknown session); caller frees. */
char *aimux_trace_session_chain(uint64_t handle, const char *session_id, AimuxError *err);

/* Query: one session's trajectory view. Returns JSON SessionTrajectory or
   NULL on failure (fills *err, e.g. unknown session); caller frees. */
char *aimux_trace_session_trajectory(uint64_t handle, const char *session_id, AimuxError *err);

/* Export all probe records as JSONL (one TraceRecord per line). Returns a
   JSON string (with embedded newlines); caller frees. */
char *aimux_trace_export_jsonl(uint64_t handle, AimuxError *err);

/* Clear all probe records of a trace handle. Returns 0, or -1 on invalid
   handle. */
int aimux_trace_clear(uint64_t handle);

/* ── Recording + mock replay (RFC-0023) ──────────────────────────────── */

/* Start recording: complete Recording JSONL is written to {dir}/recordings.jsonl
   (dir auto-created). Recording is opt-in; calling again replaces the
   recorder. Returns 0, or -1 on null dir. */
int aimux_init_recording(const char *dir);

/* Start in-memory bounded recording (RingRecorder, FIFO eviction, dropped
   count queryable). Returns 0, or -1 when cap == 0. */
int aimux_init_recording_ring(uint64_t cap);

/* No-argument variant: start in-memory bounded recording with the library
   default ring capacity (2048 entries). Ordinary callers should prefer this
   entry point; pass an explicit cap via aimux_init_recording_ring only when a
   different size is required. Returns 0. */
int aimux_init_recording_ring_default(void);

/* Stop recording: the global recorder becomes None. Returns 0. */
int aimux_recording_stop(void);

/* Flush the global recorder (blocks until JSONL is on disk; no-op for the
   ring recorder). Returns 0. */
int aimux_recording_flush(void);

/* Create a mock replay model from recorded JSONL (one Recording per line).
   Returns handle > 0 or 0 on failure (fills *err); the handle works with
   aimux_generate_text / aimux_stream_text (no real API sent); caller frees
   with aimux_free_string. */
uint64_t aimux_mock_replay_new(const char *recordings_jsonl, AimuxError *err);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* AIMUX_FFI_H */
