package ai.arcships.aimux;

import com.sun.jna.Callback;
import com.sun.jna.Library;
import com.sun.jna.Native;
import com.sun.jna.Pointer;

/**
 * JNA interface — 1:1 mapping of the aimux-ffi C ABI ({@code aimux-ffi/aimux-ffi.h}).
 *
 * <p>Error transport: fallible calls take a trailing {@link AimuxCError} (may be
 * {@code null} to discard details). Success returns a non-zero handle, owned
 * {@code char*} JSON, or non-zero stream status. Failure returns {@code 0} /
 * {@code NULL} and fills {@code *err} when non-null. No JSON error envelope on
 * the main path.
 *
 * <p>Memory ownership:
 * <ul>
 *   <li>{@code char*} returned by generate / upload / embed / search functions
 *       is owned by the caller and MUST be freed with {@link #aimux_free_string}.</li>
 *   <li>{@code const char*} passed to stream callbacks is valid only for the
 *       duration of the callback — copy it synchronously.</li>
 *   <li>{@link AimuxCError} is caller storage; nothing to free.</li>
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

    // ── Provider constructors (uint64_t handle; 0 = failure) ────────────────

    long aimux_openai_new(String apiKey, String modelId, AimuxCError err);

    long aimux_openai_new_with_base(String apiKey, String modelId, String baseUrl, AimuxCError err);

    long aimux_anthropic_new(String apiKey, String modelId, AimuxCError err);

    long aimux_anthropic_new_with_base(String apiKey, String modelId, String baseUrl, AimuxCError err);

    long aimux_cohere_new(String apiKey, String modelId, AimuxCError err);

    long aimux_cohere_new_with_base(String apiKey, String modelId, String baseUrl, AimuxCError err);

    long aimux_mistral_new(String apiKey, String modelId, AimuxCError err);

    long aimux_mistral_new_with_base(String apiKey, String modelId, String baseUrl, AimuxCError err);

    long aimux_xai_new(String apiKey, String modelId, AimuxCError err);

    long aimux_xai_new_with_base(String apiKey, String modelId, String baseUrl, AimuxCError err);

    long aimux_bedrock_new(String accessKeyId, String secretAccessKey, String region, String modelId, AimuxCError err);

    long aimux_bedrock_new_with_base(String accessKeyId, String secretAccessKey, String region, String modelId, String baseUrl, AimuxCError err);

    long aimux_vertex_new(String accessToken, String project, String location, String modelId, AimuxCError err);

    long aimux_vertex_new_with_base(String accessToken, String project, String location, String modelId, String baseUrl, AimuxCError err);

    long aimux_anthropic_aws_new(String apiKey, String region, String modelId, AimuxCError err);

    long aimux_anthropic_aws_new_with_base(String apiKey, String region, String modelId, String baseUrl, AimuxCError err);

    long aimux_azure_new(String apiKey, String resourceName, String deployment, String apiVersion, AimuxCError err);

    long aimux_azure_new_with_base(String apiKey, String baseUrl, String deployment, String apiVersion, AimuxCError err);

    // ── Registry provider (RFC-0017 phase 4) ──────────────────────────────────
    // apiKey may be null (read the provider's env var from the registry entry);
    // configJson may be null (defaults) or a JSON object of ProviderOptions.

    long aimux_provider_new(String name, String apiKey, String modelId, String configJson, AimuxCError err);

    long aimux_provider_from_env(String name, String modelId, AimuxCError err);

    // ── Provider handles (RFC-0027) ─────────────────────────────────────────

    long aimux_provider_handle_new(String name, String apiKey, String configJson, AimuxCError err);

    Pointer aimux_provider_list_models(long handle, AimuxCError err);

    long aimux_provider_model(long handle, String modelId, AimuxCError err);

    Pointer aimux_get_model_specs(String sourceUrl, AimuxCError err);

    // ── Generation ──────────────────────────────────────────────────────────

    /** @return JSON result string (caller MUST free with {@link #aimux_free_string}); NULL on failure. */
    Pointer aimux_generate_text(long handle, String promptJson, String optsJson, AimuxCError err);

    /** @return JSON GenerateObjectResult string (caller MUST free); NULL on failure. */
    Pointer aimux_generate_object(long handle, String promptJson, String optsJson, AimuxCError err);

    /** @return JSON StreamTextResultAggregated string (caller MUST free); NULL on failure. */
    Pointer aimux_consume_stream_text(long handle, String promptJson, String optsJson, AimuxCError err);

    /**
     * Push streaming with callbacks; blocks until the stream ends.
     * @return non-zero on success; 0 on failure (details in {@code err}).
     */
    int aimux_stream_text(long handle, String promptJson, String optsJson,
                          StreamPartCallback onPart, StreamDoneCallback onDone,
                          Pointer streamCtx, AimuxCError err);

    // ── OpenAI-compatible output (RFC-0026) ──────────────────────────────────

    /** @return JSON ChatCompletion string (caller MUST free with {@link #aimux_free_string}); NULL on failure. */
    Pointer aimux_generate_text_as_openai(long handle, String promptJson, String optsJson, AimuxCError err);

    /**
     * Push streaming with OpenAI Chat Completions output; blocks until the stream
     * ends. Each {@code onPart} receives a serialized ChatCompletionChunk.
     * @return non-zero on success; 0 on failure (details in {@code err}).
     */
    int aimux_stream_text_as_openai(long handle, String promptJson, String optsJson,
                                    StreamPartCallback onPart, StreamDoneCallback onDone,
                                    Pointer streamCtx, AimuxCError err);

    // ── Resource management ─────────────────────────────────────────────────

    void aimux_drop_handle(long handle);

    void aimux_free_string(Pointer ptr);

    // ── Embedding ───────────────────────────────────────────────────────────

    long aimux_openai_embedding_new(String apiKey, String modelId, AimuxCError err);

    long aimux_openai_embedding_new_with_base(String apiKey, String modelId, String baseUrl, AimuxCError err);

    long aimux_cohere_embedding_new(String apiKey, String modelId, AimuxCError err);

    long aimux_cohere_embedding_new_with_base(String apiKey, String modelId, String baseUrl, AimuxCError err);

    long aimux_google_embedding_new(String apiKey, String modelId, AimuxCError err);

    long aimux_google_embedding_new_with_base(String apiKey, String modelId, String baseUrl, AimuxCError err);

    Pointer aimux_embed(long handle, String valuesJson, String optsJson, AimuxCError err);

    // ── Speech (TTS) ────────────────────────────────────────────────────────

    long aimux_openai_speech_new(String apiKey, String modelId, AimuxCError err);

    long aimux_openai_speech_new_with_base(String apiKey, String modelId, String baseUrl, AimuxCError err);

    Pointer aimux_speech_generate(long handle, String optsJson, AimuxCError err);

    // ── Image ───────────────────────────────────────────────────────────────

    long aimux_openai_image_new(String apiKey, String modelId, AimuxCError err);

    long aimux_openai_image_new_with_base(String apiKey, String modelId, String baseUrl, AimuxCError err);

    long aimux_google_image_new(String apiKey, String modelId, AimuxCError err);

    long aimux_google_image_new_with_base(String apiKey, String modelId, String baseUrl, AimuxCError err);

    Pointer aimux_image_generate(long handle, String optsJson, AimuxCError err);

    // ── Transcription (STT, non-streaming) ──────────────────────────────────

    long aimux_openai_transcription_new(String apiKey, String modelId, AimuxCError err);

    long aimux_openai_transcription_new_with_base(String apiKey, String modelId, String baseUrl, AimuxCError err);

    Pointer aimux_transcription_generate(long handle, String audioBase64,
                                         String mediaType, String optsJson, AimuxCError err);

    // ── Files ───────────────────────────────────────────────────────────────

    long aimux_openai_files_new(String apiKey, AimuxCError err);

    long aimux_openai_files_new_with_base(String apiKey, String baseUrl, AimuxCError err);

    Pointer aimux_file_upload(long handle, String dataBase64,
                              String mediaType, String optsJson, AimuxCError err);

    // ── Reranking ───────────────────────────────────────────────────────────

    long aimux_cohere_reranking_new(String apiKey, String modelId, AimuxCError err);

    long aimux_cohere_reranking_new_with_base(String apiKey, String modelId, String baseUrl, AimuxCError err);

    Pointer aimux_rerank(long handle, String optsJson, AimuxCError err);

    // ── Video ───────────────────────────────────────────────────────────────

    long aimux_google_video_new(String apiKey, String modelId, AimuxCError err);

    long aimux_google_video_new_with_base(String apiKey, String modelId, String baseUrl, AimuxCError err);

    Pointer aimux_video_generate(long handle, String optsJson, AimuxCError err);

    // ── Search ──────────────────────────────────────────────────────────────

    long aimux_tavily_search_new(String apiKey, String modelId, AimuxCError err);

    long aimux_tavily_search_new_with_base(String apiKey, String modelId, String baseUrl, AimuxCError err);

    Pointer aimux_search(long handle, String optsJson, AimuxCError err);

    // Logging (RFC-0014).

    int aimux_init_logging(String level);

    // ── Recording + mock replay (RFC-0023) ──────────────────────────────────

    /** Start recording: complete Recording JSONL is written to {dir}/recordings.jsonl (dir auto-created). Returns 0, or -1 on null dir. */
    int aimux_init_recording(String dir);

    /** Start in-memory bounded recording (RingRecorder, FIFO eviction). Returns 0, or -1 when cap == 0. */
    int aimux_init_recording_ring(long cap);

    /** Stop recording: the global recorder becomes None. Returns 0. */
    int aimux_recording_stop();

    /** Flush the global recorder (blocks until JSONL is on disk; no-op for the ring recorder). Returns 0. */
    int aimux_recording_flush();

    /** Create a mock replay model from recorded JSONL. Handle &gt; 0 on success, 0 on failure. */
    long aimux_mock_replay_new(String recordingsJsonl, AimuxCError err);

    /** Register external OpenAI-compatible providers from JSON config (RFC-0020). Returns 1 on success, 0 on failure (fills {@code err}). */
    int aimux_register_providers(String configJson, AimuxCError err);

    /** Set the global proxy configuration (M6, RFC-0016). Returns 1 on success, 0 on failure (fills {@code err}). */
    int aimux_init_proxy(String configJson, AimuxCError err);
}
