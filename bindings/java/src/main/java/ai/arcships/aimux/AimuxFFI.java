package ai.arcships.aimux;

import com.sun.jna.Callback;
import com.sun.jna.Library;
import com.sun.jna.Native;
import com.sun.jna.Pointer;
import com.sun.jna.ptr.IntByReference;
import com.sun.jna.ptr.LongByReference;
import com.sun.jna.ptr.PointerByReference;

/**
 * JNA interface — 1:1 mapping of the aimux-ffi C ABI ({@code aimux-ffi/aimux-ffi.h}).
 *
 * <p>Error transport ({@code aimux-error.h}): every fallible call returns an
 * {@code aimux_error_t *} ({@link Pointer}) — {@code null} on success with
 * the result written to the trailing out-parameter ({@link LongByReference}
 * handle / {@link PointerByReference} owned {@code char*} JSON), non-null on
 * failure with the out-parameter left at its sentinel. Read its unified code
 * and message with {@link #aimux_error_code} / {@link #aimux_error_message},
 * then release it once
 * with {@link #aimux_error_free}. Errors are not handles: never pass one
 * to {@link #aimux_drop_handle}.
 *
 * <p>Memory ownership:
 * <ul>
 *   <li>{@code char*} returned by generate / upload / embed / search functions
 *       is owned by the caller and MUST be freed with {@link #aimux_free_string}.</li>
 *   <li>{@code const char*} passed to stream callbacks is valid only for the
 *       duration of the callback — copy it synchronously.</li>
 *   <li>Strings returned by the {@code aimux_error_*} getters are owned by the
 *       caller — free with {@link #aimux_free_string}.</li>
 * </ul>
 *
 * <p>Concurrency: all functions are synchronous (block until completion).
 * Callbacks execute on the calling thread; do NOT re-enter the FFI layer from
 * inside a callback (would deadlock the tokio runtime).
 *
 * <p>The library is resolved by JNA from {@code java.library.path} /
 * {@code LD_LIBRARY_PATH} (tests) or the JAR's {@code native/} directory.
 */
public interface AimuxFFI extends Library {

    AimuxFFI INSTANCE = Native.load("aimux_ffi", AimuxFFI.class);

    // ── Stream callbacks (match C on_part / on_done; no on_error) ───────────

    /**
     * C: {@code void (*on_part)(const char *json, void *stream_ctx)}.
     * {@code json} is valid only for the duration of the callback.
     */
    interface StreamPartCallback extends Callback {
        void invoke(Pointer json, Pointer streamCtx);
    }

    /**
     * C: {@code void (*on_done)(void *stream_ctx)}.
     * Called once on normal completion (not on failure).
     */
    interface StreamDoneCallback extends Callback {
        void invoke(Pointer streamCtx);
    }

    // ── Provider constructors (error or null; handle written to outHandle) ──

    Pointer aimux_openai_new(String apiKey, String modelId, LongByReference outHandle);

    Pointer aimux_openai_new_with_base(String apiKey, String modelId, String baseUrl, LongByReference outHandle);

    Pointer aimux_anthropic_new(String apiKey, String modelId, LongByReference outHandle);

    Pointer aimux_anthropic_new_with_base(String apiKey, String modelId, String baseUrl, LongByReference outHandle);

    Pointer aimux_cohere_new(String apiKey, String modelId, LongByReference outHandle);

    Pointer aimux_cohere_new_with_base(String apiKey, String modelId, String baseUrl, LongByReference outHandle);

    Pointer aimux_mistral_new(String apiKey, String modelId, LongByReference outHandle);

    Pointer aimux_mistral_new_with_base(String apiKey, String modelId, String baseUrl, LongByReference outHandle);

    Pointer aimux_xai_new(String apiKey, String modelId, LongByReference outHandle);

    Pointer aimux_xai_new_with_base(String apiKey, String modelId, String baseUrl, LongByReference outHandle);

    Pointer aimux_bedrock_new(String accessKeyId, String secretAccessKey, String region, String modelId, LongByReference outHandle);

    Pointer aimux_bedrock_new_with_base(String accessKeyId, String secretAccessKey, String region, String modelId, String baseUrl, LongByReference outHandle);

    Pointer aimux_vertex_new(String accessToken, String project, String location, String modelId, LongByReference outHandle);

    Pointer aimux_vertex_new_with_base(String accessToken, String project, String location, String modelId, String baseUrl, LongByReference outHandle);

    Pointer aimux_anthropic_aws_new(String apiKey, String region, String modelId, LongByReference outHandle);

    Pointer aimux_anthropic_aws_new_with_base(String apiKey, String region, String modelId, String baseUrl, LongByReference outHandle);

    Pointer aimux_azure_new(String apiKey, String resourceName, String deployment, String apiVersion, LongByReference outHandle);

    Pointer aimux_azure_new_with_base(String apiKey, String baseUrl, String deployment, String apiVersion, LongByReference outHandle);

    // ── Registry provider (RFC-0017 phase 4) ──────────────────────────────────
    // apiKey may be null (read the provider's env var from the registry entry);
    // configJson may be null (defaults) or a JSON object of ProviderOptions.

    Pointer aimux_provider_new(String name, String apiKey, String modelId, String configJson, LongByReference outHandle);

    Pointer aimux_provider_from_env(String name, String modelId, LongByReference outHandle);

    // ── Provider handles (RFC-0027) ─────────────────────────────────────────

    Pointer aimux_provider_handle_new(String name, String apiKey, String configJson, LongByReference outHandle);

    Pointer aimux_provider_list_models(long handle, PointerByReference outJson);

    Pointer aimux_provider_model(long handle, String modelId, LongByReference outHandle);

    Pointer aimux_get_model_specs(String sourceUrl, PointerByReference outJson);

    // ── Generation ──────────────────────────────────────────────────────────

    /** Writes the JSON result to {@code outJson} (caller MUST free with {@link #aimux_free_string}); returns the error or null. */
    Pointer aimux_generate_text(long handle, String promptJson, String optsJson, PointerByReference outJson);

    /** Writes JSON GenerateObjectResult to {@code outJson} (caller MUST free); returns the error or null. */
    Pointer aimux_generate_object(long handle, String promptJson, String optsJson, PointerByReference outJson);

    /** Writes JSON StreamTextResultAggregated to {@code outJson} (caller MUST free); returns the error or null. */
    Pointer aimux_consume_stream_text(long handle, String promptJson, String optsJson, PointerByReference outJson);

    /**
     * Push streaming with callbacks; blocks until the stream ends.
     * @return null after {@code onDone}; on failure the error (no {@code onDone}).
     */
    Pointer aimux_stream_text(long handle, String promptJson, String optsJson,
                              StreamPartCallback onPart, StreamDoneCallback onDone,
                              Pointer streamCtx);

    // ── OpenAI-compatible output (RFC-0026) ──────────────────────────────────

    /** Writes JSON ChatCompletion to {@code outJson} (caller MUST free with {@link #aimux_free_string}); returns the error or null. */
    Pointer aimux_generate_text_as_openai(long handle, String promptJson, String optsJson, PointerByReference outJson);

    /**
     * Push streaming with OpenAI Chat Completions output; blocks until the stream
     * ends. Each {@code onPart} receives a serialized ChatCompletionChunk.
     * @return null after {@code onDone}; on failure the error (no {@code onDone}).
     */
    Pointer aimux_stream_text_as_openai(long handle, String promptJson, String optsJson,
                                        StreamPartCallback onPart, StreamDoneCallback onDone,
                                        Pointer streamCtx);

    // ── Resource management ─────────────────────────────────────────────────

    void aimux_drop_handle(long handle);

    void aimux_free_string(Pointer ptr);

    // ── Returned errors (aimux-error.h) ─────────────────────────────────────

    /** Release the error returned by a failed call. NULL-safe; exactly once. */
    void aimux_error_free(Pointer err);

    // ── Unified code and getters (const aimux_error_t *; NULL-safe) ─────────

    /** {@code aimux_error_code_t}; {@link AimuxException#AIMUX_OK} for NULL. */
    int aimux_error_code(Pointer err);

    /** Owned UTF-8 message (free with {@link #aimux_free_string}); NULL only for NULL. */
    Pointer aimux_error_message(Pointer err);

    /** 1 when retrying may help (only ever 1 for AIMUX_E_API_CALL). */
    int aimux_error_retryable(Pointer err);

    /** AIMUX_E_API_CALL: HTTP status or -1. */
    int aimux_error_status(Pointer err);

    /** AIMUX_E_API_CALL: rate-limit hint in ms, or -1. */
    long aimux_error_retry_ms(Pointer err);

    /** AIMUX_E_API_CALL: owned string or NULL. */
    Pointer aimux_error_provider_code(Pointer err);

    /** AIMUX_E_API_CALL: owned string or NULL. */
    Pointer aimux_error_provider_message(Pointer err);

    /** AIMUX_E_API_CALL: owned string or NULL. */
    Pointer aimux_error_request_id(Pointer err);

    /** AIMUX_E_API_CALL: owned string or NULL. */
    Pointer aimux_error_response_body(Pointer err);

    /** AIMUX_E_NO_SUCH_MODEL: owned string or NULL. */
    Pointer aimux_error_model_id(Pointer err);

    /** AIMUX_E_NO_SUCH_MODEL: owned string or NULL. */
    Pointer aimux_error_model_type(Pointer err);

    /** AIMUX_E_NO_SUCH_PROVIDER: owned string or NULL. */
    Pointer aimux_error_provider_id(Pointer err);

    // ── Embedding ───────────────────────────────────────────────────────────

    Pointer aimux_openai_embedding_new(String apiKey, String modelId, LongByReference outHandle);

    Pointer aimux_openai_embedding_new_with_base(String apiKey, String modelId, String baseUrl, LongByReference outHandle);

    Pointer aimux_cohere_embedding_new(String apiKey, String modelId, LongByReference outHandle);

    Pointer aimux_cohere_embedding_new_with_base(String apiKey, String modelId, String baseUrl, LongByReference outHandle);

    Pointer aimux_google_embedding_new(String apiKey, String modelId, LongByReference outHandle);

    Pointer aimux_google_embedding_new_with_base(String apiKey, String modelId, String baseUrl, LongByReference outHandle);

    Pointer aimux_embed(long handle, String valuesJson, String optsJson, PointerByReference outJson);

    // ── Speech (TTS) ────────────────────────────────────────────────────────

    Pointer aimux_openai_speech_new(String apiKey, String modelId, LongByReference outHandle);

    Pointer aimux_openai_speech_new_with_base(String apiKey, String modelId, String baseUrl, LongByReference outHandle);

    Pointer aimux_speech_generate(long handle, String optsJson, PointerByReference outJson);

    // ── Image ───────────────────────────────────────────────────────────────

    Pointer aimux_openai_image_new(String apiKey, String modelId, LongByReference outHandle);

    Pointer aimux_openai_image_new_with_base(String apiKey, String modelId, String baseUrl, LongByReference outHandle);

    Pointer aimux_google_image_new(String apiKey, String modelId, LongByReference outHandle);

    Pointer aimux_google_image_new_with_base(String apiKey, String modelId, String baseUrl, LongByReference outHandle);

    Pointer aimux_image_generate(long handle, String optsJson, PointerByReference outJson);

    // ── Transcription (STT, non-streaming) ──────────────────────────────────

    Pointer aimux_openai_transcription_new(String apiKey, String modelId, LongByReference outHandle);

    Pointer aimux_openai_transcription_new_with_base(String apiKey, String modelId, String baseUrl, LongByReference outHandle);

    Pointer aimux_transcription_generate(long handle, String audioBase64,
                                         String mediaType, String optsJson, PointerByReference outJson);

    // ── Files ───────────────────────────────────────────────────────────────

    Pointer aimux_openai_files_new(String apiKey, LongByReference outHandle);

    Pointer aimux_openai_files_new_with_base(String apiKey, String baseUrl, LongByReference outHandle);

    Pointer aimux_file_upload(long handle, String dataBase64,
                              String mediaType, String optsJson, PointerByReference outJson);

    // ── Reranking ───────────────────────────────────────────────────────────

    Pointer aimux_cohere_reranking_new(String apiKey, String modelId, LongByReference outHandle);

    Pointer aimux_cohere_reranking_new_with_base(String apiKey, String modelId, String baseUrl, LongByReference outHandle);

    Pointer aimux_rerank(long handle, String optsJson, PointerByReference outJson);

    // ── Video ───────────────────────────────────────────────────────────────

    Pointer aimux_google_video_new(String apiKey, String modelId, LongByReference outHandle);

    Pointer aimux_google_video_new_with_base(String apiKey, String modelId, String baseUrl, LongByReference outHandle);

    Pointer aimux_video_generate(long handle, String optsJson, PointerByReference outJson);

    // ── Search ──────────────────────────────────────────────────────────────

    Pointer aimux_tavily_search_new(String apiKey, String modelId, LongByReference outHandle);

    Pointer aimux_tavily_search_new_with_base(String apiKey, String modelId, String baseUrl, LongByReference outHandle);

    Pointer aimux_search(long handle, String optsJson, PointerByReference outJson);

    // Logging (RFC-0014). C ABI failures only.

    Pointer aimux_init_logging(String level);

    // ── Recording + mock replay (RFC-0023) ──────────────────────────────────

    /** Start recording: complete Recording JSONL is written to {dir}/recordings.jsonl (dir auto-created). Returns null, or a RecordingError code (100–102). */
    Pointer aimux_init_recording(String dir);

    /** Start in-memory bounded recording (RingRecorder, FIFO eviction). cap == 0 is AIMUX_E_INVALID_ARGUMENT. */
    Pointer aimux_init_recording_ring(long cap);

    /** Stop recording: the global recorder becomes None. Infallible. */
    void aimux_recording_stop();

    /** Flush the global recorder (blocks until JSONL is on disk; no-op for the ring recorder). Infallible. */
    void aimux_recording_flush();

    /** Checked flush: null = data on disk (or nothing recording); otherwise a RecordingError code (read with {@link #aimux_error_code} / {@link #aimux_error_message}, release with {@link #aimux_error_free}). */
    Pointer aimux_recording_try_flush();

    /** Create a mock replay model from recorded JSONL. Writes the handle to {@code outHandle}; returns the error or null. */
    Pointer aimux_mock_replay_new(String recordingsJsonl, LongByReference outHandle);

    /** Register external OpenAI-compatible providers from JSON config (RFC-0020). Returns the error or null. */
    Pointer aimux_register_providers(String configJson);

    /** Set the global proxy configuration (M6, RFC-0016). Returns the error or null. */
    Pointer aimux_init_proxy(String configJson);

    /** Create a RouterModel (RFC-0021) over child handles. {@code handles} is pinned by
     *  JNA to a {@code uint64_t*}; unknown handles are dropped. {@code configJson} may be
     *  null. Writes the handle to {@code outHandle}; returns the error or null. */
    Pointer aimux_router_new(long[] handles, long len, String configJson, LongByReference outHandle);

    /** Create a MoaModel (RFC-0022) over reference handles + one aggregator handle.
     *  {@code referenceHandles} may be null/empty (degrades to aggregator-only). Writes
     *  the handle to {@code outHandle}; returns the error or null. */
    Pointer aimux_moa_new(long[] referenceHandles, long refLen, long aggregator, String configJson, LongByReference outHandle);

    // ── Transcription streaming sessions (RFC-0028) ─────────────────────────

    /** Start a streaming transcription session. {@code abortHandle} 0 = none.
     *  {@code optsJson} nullable. Writes the session handle to {@code outHandle}; returns the error or null. */
    Pointer aimux_transcription_session_new(long modelHandle, long abortHandle, String optsJson, LongByReference outHandle);

    /** Push one binary audio chunk (BLOCKS while the channel is full). Returns the error or null. */
    Pointer aimux_transcription_push_audio(long session, byte[] data, long len);

    /** Signal end-of-audio (idempotent). Returns the error or null (a dead handle is a C ABI failure). */
    Pointer aimux_transcription_input_done(long session);

    /** Values written to {@code outState} (an {@code int32_t}, hence {@code IntByReference}). */
    int TRANSCRIPTION_NEXT_PART_PART = 1;
    int TRANSCRIPTION_NEXT_PART_ENDED = 2;
    int TRANSCRIPTION_NEXT_PART_TIMEOUT = 3;

    /** Pull the next part. On a null return {@code outState} says what happened:
     *  PART ({@code outPart} holds JSON, free with {@link #aimux_free_string}), ENDED or
     *  TIMEOUT ({@code outPart} NULL). A non-null return has an AiMuxError code on
     *  abort / API failure or a C ABI failure code on a dead handle. */
    Pointer aimux_transcription_next_part(long session, long timeoutMs, PointerByReference outPart, IntByReference outState);

    /** Terminate and release the session (safe with 0). */
    void aimux_transcription_session_drop(long session);
}
