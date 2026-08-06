/**
 * aimux-ffi.h — C ABI for aimux multi-language bindings.
 *
 * This is the C ABI boundary layer. Only used by C ABI bindings
 * (Swift / Kotlin / C / C++). Native bindings (Python / Node / Flutter)
 * bypass this layer and use aimux-providers directly.
 *
 * ## Memory ownership
 *
 * Every function that returns `char*` — constructors included — returns a
 * string owned by the caller; the caller MUST free it with
 * `aimux_free_string`. A `char*`-returning function returns NULL only if the
 * result string could not be allocated; `aimux_free_string(NULL)` is safe.
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
 * milliseconds; the latter two streaming-only) is the only way to bound a
 * call — there is no abort/cancel entry point over the C ABI. Without it a
 * hung provider blocks the calling thread indefinitely.
 *
 * ## Wire format
 *
 * - Errors: every failure is the envelope
 *   `{"error":"...","error_type":"...","status_code":<u16|null>}` —
 *   constructors, `aimux_generate_text`, `on_error`, all of them.
 * - `prompt_json`: bare prompt value (`"text"` or `[{...}]`), or a
 *   single-key wrapper `{"prompt": <value>}` (any extra key disables
 *   unwrapping and the whole object is parsed as the prompt)
 * - `opts_json` for `aimux_generate_text`/`aimux_stream_text`: serialized
 *   GenerateTextOptions (empty/null for defaults). Multimodal calls
 *   (`aimux_speech_generate`, `aimux_image_generate`, `aimux_rerank`,
 *   `aimux_video_generate`, `aimux_search`) REQUIRE a valid JSON object —
 *   NULL or empty returns an error envelope.
 * - Results: serialized JSON of GenerateTextResult, or an error envelope
 * - Stream parts: serialized JSON of StreamPart
 * - Constructors: `{"handle":<u64>}` on success, or an error envelope
 */

#ifndef AIMUX_FFI_H
#define AIMUX_FFI_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ── Provider constructors ──────────────────────────────────────────────── */

/**
 * Create an OpenAI model instance.
 *
 * @param api_key  NUL-terminated API key string.
 * @param model_id NUL-terminated model ID (e.g. "gpt-4o").
 * @return JSON string (caller MUST free with aimux_free_string):
 *         `{"handle":<u64>}` on success, `{"error":...}` on failure.
 */
char *aimux_openai_new(const char *api_key, const char *model_id);

/**
 * Create an OpenAI model instance with a custom base URL.
 *
 * @param api_key  NUL-terminated API key string.
 * @param model_id NUL-terminated model ID (e.g. "gpt-4o").
 * @param base_url NUL-terminated base URL (NULL or empty uses the provider default).
 * @return JSON string (caller MUST free with aimux_free_string):
 *         `{"handle":<u64>}` on success, `{"error":...}` on failure.
 */
char *aimux_openai_new_with_base(const char *api_key,
                                 const char *model_id,
                                 const char *base_url);

/**
 * Create an Anthropic model instance.
 *
 * @param api_key  NUL-terminated API key string.
 * @param model_id NUL-terminated model ID (e.g. "claude-3-5-sonnet-20241022").
 * @return JSON string (caller MUST free with aimux_free_string):
 *         `{"handle":<u64>}` on success, `{"error":...}` on failure.
 */
char *aimux_anthropic_new(const char *api_key, const char *model_id);

/**
 * Create an Anthropic model instance with a custom base URL.
 *
 * @param api_key  NUL-terminated API key string.
 * @param model_id NUL-terminated model ID (e.g. "claude-3-5-sonnet-20241022").
 * @param base_url NUL-terminated base URL (NULL or empty uses the provider default).
 * @return JSON string (caller MUST free with aimux_free_string):
 *         `{"handle":<u64>}` on success, `{"error":...}` on failure.
 */
char *aimux_anthropic_new_with_base(const char *api_key,
                                    const char *model_id,
                                    const char *base_url);

/**
 * Create a Cohere model instance (API key + model ID). Returns a JSON
 * string (caller MUST free with aimux_free_string): `{"handle":<u64>}` on
 * success, `{"error":...}` on failure. `_with_base` adds `base_url`
 * (NULL or empty uses the provider default).
 */
char *aimux_cohere_new(const char *api_key, const char *model_id);
char *aimux_cohere_new_with_base(const char *api_key,
                                 const char *model_id,
                                 const char *base_url);

/**
 * Create a Mistral model instance (API key + model ID). Returns a JSON
 * string (caller MUST free with aimux_free_string): `{"handle":<u64>}` on
 * success, `{"error":...}` on failure. `_with_base` adds `base_url`
 * (NULL or empty uses the provider default).
 */
char *aimux_mistral_new(const char *api_key, const char *model_id);
char *aimux_mistral_new_with_base(const char *api_key,
                                  const char *model_id,
                                  const char *base_url);

/**
 * Create an xAI model instance (API key + model ID). Returns a JSON
 * string (caller MUST free with aimux_free_string): `{"handle":<u64>}` on
 * success, `{"error":...}` on failure. `_with_base` adds `base_url`
 * (NULL or empty uses the provider default).
 */
char *aimux_xai_new(const char *api_key, const char *model_id);
char *aimux_xai_new_with_base(const char *api_key,
                              const char *model_id,
                              const char *base_url);

/**
 * Create a Bedrock model instance (AWS SigV4 credentials). Returns a JSON
 * string (caller MUST free with aimux_free_string): `{"handle":<u64>}` on
 * success, `{"error":...}` on failure. `_with_base` adds `base_url`
 * (NULL or empty uses the provider default).
 */
char *aimux_bedrock_new(const char *access_key_id,
                        const char *secret_access_key,
                        const char *region,
                        const char *model_id);
char *aimux_bedrock_new_with_base(const char *access_key_id,
                                  const char *secret_access_key,
                                  const char *region,
                                  const char *model_id,
                                  const char *base_url);

/**
 * Create a Vertex AI model instance (GCP bearer token). Returns a JSON
 * string (caller MUST free with aimux_free_string): `{"handle":<u64>}` on
 * success, `{"error":...}` on failure. `_with_base` adds `base_url`
 * (NULL or empty uses the provider default).
 */
char *aimux_vertex_new(const char *access_token,
                       const char *project,
                       const char *location,
                       const char *model_id);
char *aimux_vertex_new_with_base(const char *access_token,
                                 const char *project,
                                 const char *location,
                                 const char *model_id,
                                 const char *base_url);

/**
 * Create an Anthropic-on-AWS model instance (API key + region). Returns a
 * JSON string (caller MUST free with aimux_free_string): `{"handle":<u64>}`
 * on success, `{"error":...}` on failure. `_with_base` adds `base_url`
 * (NULL or empty uses the provider default).
 */
char *aimux_anthropic_aws_new(const char *api_key, const char *region,
                              const char *model_id);
char *aimux_anthropic_aws_new_with_base(const char *api_key, const char *region,
                                        const char *model_id, const char *base_url);

/**
 * Create an Azure OpenAI model instance (API key + resource name; deployment
 * passed as model_id; api_version NULL or empty uses the provider default).
 * Returns a JSON string (caller MUST free with aimux_free_string):
 * `{"handle":<u64>}` on success, `{"error":...}` on failure.
 *
 * `_with_base` takes an explicit `base_url` IN PLACE OF `resource_name`.
 * Unlike other `_with_base` variants, `base_url` here is REQUIRED — NULL
 * returns an InvalidArgument error envelope.
 */
char *aimux_azure_new(const char *api_key, const char *resource_name,
                      const char *deployment, const char *api_version);
char *aimux_azure_new_with_base(const char *api_key, const char *base_url,
                                const char *deployment, const char *api_version);

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
 * @return JSON string (caller MUST free with aimux_free_string):
 *         `{"handle":<u64>}` on success, `{"error":...}` on failure (unknown
 *         provider, bad config, missing env key, or invalid model id).
 */
char *aimux_provider_new(const char *name, const char *api_key,
                         const char *model_id, const char *config_json);

/**
 * Convenience: create a model by provider name, reading the API key from the
 * provider's env var. Returns a JSON string (caller MUST free with
 * aimux_free_string): `{"handle":<u64>}` on success, `{"error":...}` on
 * failure.
 */
char *aimux_provider_from_env(const char *name, const char *model_id);

/**
 * Provider handles (RFC-0027): create a provider handle for a registry-backed
 * provider. Unlike aimux_provider_new (which binds to a single model_id),
 * this returns a provider handle supporting aimux_provider_list_models and
 * aimux_provider_model. Returns `{"handle":<u64>}` or `{"error":...}`.
 */
char *aimux_provider_handle_new(const char *name, const char *api_key,
                                const char *config_json);

/**
 * List models on a provider handle (runtime discovery + anya2a enrichment).
 * Returns a JSON array of ResolvedModel, or `{"error":...}`.
 */
char *aimux_provider_list_models(uint64_t handle);

/**
 * Build a language model from a provider handle + model_id.
 * Returns `{"handle":<u64>}` or `{"error":...}`.
 */
char *aimux_provider_model(uint64_t handle, const char *model_id);

/* ── Generation ─────────────────────────────────────────────────────────── */

/**
 * Non-streaming text generation.
 *
 * @param handle      Language-model handle from aimux_*_new's `{"handle":<u64>}`
 *                    (a handle of another modality yields "invalid handle").
 * @param prompt_json JSON prompt (see wire format above).
 * @param opts_json   JSON options (NULL or empty for defaults).
 * @return JSON result string (caller MUST free with aimux_free_string).
 *         On error: the `{"error","error_type","status_code"}` envelope.
 *         Malformed input returns `{"error":"invalid prompt_json: <detail>",
 *         "error_type":"Other","status_code":null}` (same for opts_json)
 *         before any network call.
 */
char *aimux_generate_text(uint64_t handle,
                          const char *prompt_json,
                          const char *opts_json);

/**
 * Streaming text generation with push callbacks (blocks until stream ends).
 *
 * @param handle      Model handle from aimux_*_new's `{"handle":<u64>}`.
 * @param prompt_json JSON prompt (see wire format above).
 * @param opts_json   JSON options (NULL or empty for defaults).
 * @param on_part     Called for each StreamPart (JSON string, valid during call only).
 * @param on_done     Called once when the stream ends normally.
 * @param on_error    Called on a stream-level error (JSON `{"error":"..."}` string).
 */
void aimux_stream_text(uint64_t handle,
                       const char *prompt_json,
                       const char *opts_json,
                       void (*on_part)(const char *json),
                       void (*on_done)(void),
                       void (*on_error)(const char *err_json));

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
 * aimux_abort_signal_abort while this function runs. Cancellation calls
 * on_error with an Aborted error and does not call on_done.
 *
 * @param handle       Model handle from aimux_*_new.
 * @param abort_handle Handle from aimux_abort_signal_new.
 * @param prompt_json  JSON prompt.
 * @param opts_json    JSON options. NULL or empty uses defaults.
 * @param on_part      Called for each StreamPart.
 * @param on_done      Called once after normal completion.
 * @param on_error     Called once after an error or cancellation.
 */
void aimux_stream_text_with_abort(uint64_t handle,
                                  uint64_t abort_handle,
                                  const char *prompt_json,
                                  const char *opts_json,
                                  void (*on_part)(const char *json),
                                  void (*on_done)(void),
                                  void (*on_error)(const char *err_json));

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
                                    const char *opts_json);

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
void aimux_stream_text_as_openai(uint64_t handle,
                                 const char *prompt_json,
                                 const char *opts_json,
                                 void (*on_part)(const char *json),
                                 void (*on_done)(void),
                                 void (*on_error)(const char *err_json));

/**
 * Cancelable streaming OpenAI-compatible output (see aimux_stream_text_with_abort).
 */
void aimux_stream_text_as_openai_with_abort(uint64_t handle,
                                            uint64_t abort_handle,
                                            const char *prompt_json,
                                            const char *opts_json,
                                            void (*on_part)(const char *json),
                                            void (*on_done)(void),
                                            void (*on_error)(const char *err_json));

/* ── Resource management ────────────────────────────────────────────────── */

/**
 * Release a model handle. Safe to call with 0 (no-op).
 *
 * @param handle Model handle from aimux_*_new's `{"handle":<u64>}`.
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
/* Constructors below return the `{"handle":<u64>}` / error envelope;
   `*_generate`-style calls return the modality's Result JSON or an error
   envelope. All returned strings must be freed with aimux_free_string. */

char *aimux_openai_embedding_new(const char *api_key, const char *model_id);
char *aimux_openai_embedding_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_cohere_embedding_new(const char *api_key, const char *model_id);
char *aimux_cohere_embedding_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_google_embedding_new(const char *api_key, const char *model_id);
char *aimux_google_embedding_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_embed(uint64_t handle, const char *values_json, const char *opts_json);

/* ── Speech (TTS) ────────────────────────────────────────────────────────── */

char *aimux_openai_speech_new(const char *api_key, const char *model_id);
char *aimux_openai_speech_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_speech_generate(uint64_t handle, const char *opts_json);

/* ── Image ──────────────────────────────────────────────────────────────── */

char *aimux_openai_image_new(const char *api_key, const char *model_id);
char *aimux_openai_image_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_google_image_new(const char *api_key, const char *model_id);
char *aimux_google_image_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_image_generate(uint64_t handle, const char *opts_json);

/* ── Transcription (STT, non-streaming) ──────────────────────────────────── */

char *aimux_openai_transcription_new(const char *api_key, const char *model_id);
char *aimux_openai_transcription_new_with_base(const char *api_key, const char *model_id, const char *base_url);
/* opts_json is currently IGNORED (reserved for future options). */
char *aimux_transcription_generate(uint64_t handle, const char *audio_base64, const char *media_type, const char *opts_json);

/* ── Files ──────────────────────────────────────────────────────────────── */

char *aimux_openai_files_new(const char *api_key);
char *aimux_openai_files_new_with_base(const char *api_key, const char *base_url);
/* opts_json is currently IGNORED (reserved for future options). */
char *aimux_file_upload(uint64_t handle, const char *data_base64, const char *media_type, const char *opts_json);

/* ── Reranking ───────────────────────────────────────────────────────────── */

char *aimux_cohere_reranking_new(const char *api_key, const char *model_id);
char *aimux_cohere_reranking_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_rerank(uint64_t handle, const char *opts_json);

/* ── Video ───────────────────────────────────────────────────────────────── */

char *aimux_google_video_new(const char *api_key, const char *model_id);
char *aimux_google_video_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_video_generate(uint64_t handle, const char *opts_json);

/* ── Search ──────────────────────────────────────────────────────────────── */

/* model_id is accepted for API symmetry but ignored (Tavily uses a fixed
   endpoint). */
char *aimux_tavily_search_new(const char *api_key, const char *model_id);
char *aimux_tavily_search_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_search(uint64_t handle, const char *opts_json);

/* Codex (RFC-0018) */

/* Refresh a Codex subscription access token (stateless OAuth helper).
   Returns JSON {"access_token","refresh_token","expires_in_secs"} or error
   JSON; caller frees with aimux_free_string. Caller owns token persistence
   and the 401 -> refresh -> retry orchestration. */
char *aimux_codex_refresh(const char *refresh_token, const char *client_id);

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
   SessionCall[] (empty if unknown / no store) or {"error":...}; caller frees
   with aimux_free_string. */
char *aimux_session_calls(const char *session_id);

/* Query: all known sessions. Returns a JSON SessionView[] or
   {"error":...}; caller frees with aimux_free_string. */
char *aimux_list_sessions(void);

/* Cache probing (RFC-0015) */

/* Wrap a model handle in a probe layer. The returned handle works with
   aimux_generate_text / aimux_stream_text (probed) and the aimux_trace_*
   queries. Returns {"handle":<u64>} or {"error":...}; caller frees. */
char *aimux_trace_new(uint64_t handle);

/* Same, with the built-in rules auditor attached. strict nonzero = strict
   mode; zero = shared (safe default). */
char *aimux_trace_new_audited(uint64_t handle, int strict);

/* Query: aggregated probe statistics, filtered by filter_json (serialized
   TraceFilter; NULL = all). Returns JSON TraceStats[]; caller frees. */
char *aimux_trace_aggregate(uint64_t handle, const char *filter_json);

/* Query: one session's chain view. Returns JSON SessionChainView or
   {"error":"unknown session"}; caller frees. */
char *aimux_trace_session_chain(uint64_t handle, const char *session_id);

/* Export all probe records as JSONL (one TraceRecord per line). Returns a
   JSON string (with embedded newlines); caller frees. */
char *aimux_trace_export_jsonl(uint64_t handle);

/* Clear all probe records of a trace handle. Returns 0, or -1 on invalid
   handle. */
int aimux_trace_clear(uint64_t handle);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* AIMUX_FFI_H */
