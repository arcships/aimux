/**
 * aimux-ffi.h — C ABI for aimux multi-language bindings.
 *
 * This is the C ABI boundary layer. Only used by C ABI bindings
 * (Swift / Kotlin / C / C++). Native bindings (Python / Node / Flutter)
 * bypass this layer and use aimux-providers directly.
 *
 * ## Memory ownership
 *
 * - `aimux_generate_text` returns a `char*` owned by the caller; the caller
 *   MUST free it with `aimux_free_string`.
 * - `aimux_stream_text` callbacks receive `const char*` pointers that are
 *   valid **only for the duration of the callback**. The callback must copy
 *   the data synchronously.
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
 * @return Opaque handle (>0 on success, 0 on failure).
 */
uint64_t aimux_openai_new(const char *api_key, const char *model_id);

/**
 * Create an OpenAI model instance with a custom base URL.
 *
 * @param api_key  NUL-terminated API key string.
 * @param model_id NUL-terminated model ID (e.g. "gpt-4o").
 * @param base_url NUL-terminated base URL (NULL or empty uses the provider default).
 * @return Opaque handle (>0 on success, 0 on failure).
 */
uint64_t aimux_openai_new_with_base(const char *api_key,
                                    const char *model_id,
                                    const char *base_url);

/**
 * Create an Anthropic model instance.
 *
 * @param api_key  NUL-terminated API key string.
 * @param model_id NUL-terminated model ID (e.g. "claude-3-5-sonnet-20241022").
 * @return Opaque handle (>0 on success, 0 on failure).
 */
uint64_t aimux_anthropic_new(const char *api_key, const char *model_id);

/**
 * Create an Anthropic model instance with a custom base URL.
 *
 * @param api_key  NUL-terminated API key string.
 * @param model_id NUL-terminated model ID (e.g. "claude-3-5-sonnet-20241022").
 * @param base_url NUL-terminated base URL (NULL or empty uses the provider default).
 * @return Opaque handle (>0 on success, 0 on failure).
 */
uint64_t aimux_anthropic_new_with_base(const char *api_key,
                                       const char *model_id,
                                       const char *base_url);

/* ── Generation ─────────────────────────────────────────────────────────── */

/**
 * Non-streaming text generation.
 *
 * @param handle      Model handle from aimux_*_new.
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
 * @param handle      Model handle from aimux_*_new.
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
 * @param handle Model handle from aimux_*_new.
 */
void aimux_drop_handle(uint64_t handle);

/**
 * Free a string previously returned by aimux_generate_text or other
 * functions returning char*.
 *
 * @param ptr Pointer from an aimux_* function (NULL is safe).
 */
void aimux_free_string(char *ptr);

/* ── Embedding ───────────────────────────────────────────────────────────── */

uint64_t aimux_openai_embedding_new(const char *api_key, const char *model_id);
char *aimux_embed(uint64_t handle, const char *values_json, const char *opts_json);

/* ── Speech (TTS) ────────────────────────────────────────────────────────── */

uint64_t aimux_openai_speech_new(const char *api_key, const char *model_id);
char *aimux_speech_generate(uint64_t handle, const char *opts_json);

/* ── Image ──────────────────────────────────────────────────────────────── */

uint64_t aimux_openai_image_new(const char *api_key, const char *model_id);
char *aimux_image_generate(uint64_t handle, const char *opts_json);

/* ── Transcription (STT, non-streaming) ──────────────────────────────────── */

uint64_t aimux_openai_transcription_new(const char *api_key, const char *model_id);
char *aimux_transcription_generate(uint64_t handle, const char *audio_base64, const char *media_type, const char *opts_json);

/* ── Files ──────────────────────────────────────────────────────────────── */

uint64_t aimux_openai_files_new(const char *api_key);
char *aimux_file_upload(uint64_t handle, const char *data_base64, const char *media_type, const char *opts_json);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* AIMUX_FFI_H */
