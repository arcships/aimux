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
 * `aimux_free_string`. `aimux_stream_text` callbacks receive `const char*`
 * pointers that are valid **only for the duration of the callback**. The
 * callback must copy the data synchronously.
 *
 * ## Concurrency
 *
 * All functions are synchronous (block until the operation completes).
 * Callbacks execute on the same thread; do not re-enter the FFI layer
 * from a callback (would deadlock the tokio runtime).
 *
 * ## Wire format
 *
 * - `prompt_json`: bare prompt value (`"text"` or `[{...}]`) or `{"prompt": <value>}`
 * - `opts_json`: serialized GenerateTextOptions (empty/null for defaults)
 * - Results: serialized JSON of GenerateTextResult, or `{"error":"..."}`
 * - Stream parts: serialized JSON of StreamPart
 * - Constructors: `{"handle":<u64>}` on success, or
 *   `{"error":"...","error_type":"...","status_code":<u16|null>}` on failure
 *   (the same envelope shape `aimux_generate_text` returns on failure).
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
 * success, `{"error":...}` on failure.
 */
char *aimux_cohere_new(const char *api_key, const char *model_id);
char *aimux_cohere_new_with_base(const char *api_key,
                                 const char *model_id,
                                 const char *base_url);

/**
 * Create a Mistral model instance (API key + model ID). Returns a JSON
 * string (caller MUST free with aimux_free_string): `{"handle":<u64>}` on
 * success, `{"error":...}` on failure.
 */
char *aimux_mistral_new(const char *api_key, const char *model_id);
char *aimux_mistral_new_with_base(const char *api_key,
                                  const char *model_id,
                                  const char *base_url);

/**
 * Create an xAI model instance (API key + model ID). Returns a JSON
 * string (caller MUST free with aimux_free_string): `{"handle":<u64>}` on
 * success, `{"error":...}` on failure.
 */
char *aimux_xai_new(const char *api_key, const char *model_id);
char *aimux_xai_new_with_base(const char *api_key,
                              const char *model_id,
                              const char *base_url);

/**
 * Create a Bedrock model instance (AWS SigV4 credentials). Returns a JSON
 * string (caller MUST free with aimux_free_string): `{"handle":<u64>}` on
 * success, `{"error":...}` on failure.
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
 * success, `{"error":...}` on failure.
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
 * on success, `{"error":...}` on failure.
 */
char *aimux_anthropic_aws_new(const char *api_key, const char *region,
                              const char *model_id);
char *aimux_anthropic_aws_new_with_base(const char *api_key, const char *region,
                                        const char *model_id, const char *base_url);

/**
 * Create an Azure OpenAI model instance (API key + resource name; deployment
 * passed as model_id; api_version NULL uses the provider default). Returns a
 * JSON string (caller MUST free with aimux_free_string): `{"handle":<u64>}`
 * on success, `{"error":...}` on failure.
 */
char *aimux_azure_new(const char *api_key, const char *resource_name,
                      const char *deployment, const char *api_version);
char *aimux_azure_new_with_base(const char *api_key, const char *base_url,
                                const char *deployment, const char *api_version);

/**
 * Create a model from the provider registry by name (RFC-0017 phase 4).
 *
 * @param name        NUL-terminated registry provider name (e.g. "deepseek", "groq").
 * @param api_key     NUL-terminated API key, or NULL to read the provider's
 *                    env var from the registry entry.
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

/* ── Generation ─────────────────────────────────────────────────────────── */

/**
 * Non-streaming text generation.
 *
 * @param handle      Model handle from aimux_*_new's `{"handle":<u64>}`.
 * @param prompt_json JSON prompt (see wire format above).
 * @param opts_json   JSON options (NULL or empty for defaults).
 * @return JSON result string (caller MUST free with aimux_free_string).
 *         On error: `{"error":"..."}`.
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

/**
 * Take (read-and-clear) the last constructor error on this thread.
 *
 * Constructors return a bare handle with 0 reserved for failure, so callers
 * cannot distinguish "unknown provider" from "bad config" from "missing env
 * var". On failure, the constructor records the full error JSON envelope
 * `{"error","error_type","status_code"}`; callers read it back with this
 * function and feed it through their existing error-JSON parser.
 *
 * Semantics:
 * - Returns NULL if the last constructor call on this thread succeeded.
 * - Destructive read: a second call returns NULL.
 * - The returned pointer is owned by the caller; MUST be freed with
 *   aimux_free_string.
 * - Only set by aimux_*_new / aimux_provider_new / aimux_provider_from_env.
 *
 * Threading: the constructor and this read must run on the SAME OS thread,
 * with no other aimux_*_new call in between. Runtimes that can migrate work
 * between OS threads between native calls (Go goroutines, Java virtual
 * threads, Kotlin coroutines) must pin both calls to one thread.
 *
 * @return Error JSON envelope, or NULL (no error).
 */
char *aimux_last_error(void);

/* ── Embedding ───────────────────────────────────────────────────────────── */

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
char *aimux_transcription_generate(uint64_t handle, const char *audio_base64, const char *media_type, const char *opts_json);

/* ── Files ──────────────────────────────────────────────────────────────── */

char *aimux_openai_files_new(const char *api_key);
char *aimux_openai_files_new_with_base(const char *api_key, const char *base_url);
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

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* AIMUX_FFI_H */

extern void *const aimux_ffi_all_symbols[];
