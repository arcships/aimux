package io.aimux;

import com.sun.jna.Callback;
import com.sun.jna.Library;
import com.sun.jna.Native;
import com.sun.jna.Pointer;

/**
 * JNA interface — 1:1 mapping of the aimux-ffi C ABI (aimux-ffi/aimux-ffi.h).
 *
 * <p>Memory ownership follows the C contract:
 * <ul>
 *   <li>{@code char*} returned by every {@code *_generate} / {@code *_upload} /
 *       {@code *_embed} / {@code *search} function is owned by the caller and
 *       MUST be freed with {@code aimux_free_string}.</li>
 *   <li>{@code const char*} passed to stream callbacks is valid only for the
 *       duration of the callback — copy it synchronously.</li>
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

    // ── Provider constructors (return opaque handle, >0 on success, 0 on failure) ──

    long aimux_openai_new(String apiKey, String modelId);

    long aimux_openai_new_with_base(String apiKey, String modelId, String baseUrl);

    long aimux_anthropic_new(String apiKey, String modelId);

    long aimux_anthropic_new_with_base(String apiKey, String modelId, String baseUrl);

    long aimux_cohere_new(String apiKey, String modelId);

    long aimux_cohere_new_with_base(String apiKey, String modelId, String baseUrl);

    long aimux_mistral_new(String apiKey, String modelId);

    long aimux_mistral_new_with_base(String apiKey, String modelId, String baseUrl);

    long aimux_xai_new(String apiKey, String modelId);

    long aimux_xai_new_with_base(String apiKey, String modelId, String baseUrl);

    long aimux_bedrock_new(String accessKeyId, String secretAccessKey, String region, String modelId);

    long aimux_bedrock_new_with_base(String accessKeyId, String secretAccessKey, String region, String modelId, String baseUrl);

    long aimux_vertex_new(String accessToken, String project, String location, String modelId);

    long aimux_vertex_new_with_base(String accessToken, String project, String location, String modelId, String baseUrl);

    long aimux_anthropic_aws_new(String apiKey, String region, String modelId);

    long aimux_anthropic_aws_new_with_base(String apiKey, String region, String modelId, String baseUrl);

    long aimux_azure_new(String apiKey, String resourceName, String deployment, String apiVersion);

    long aimux_azure_new_with_base(String apiKey, String baseUrl, String deployment, String apiVersion);

    // ── Registry provider (RFC-0017 phase 4) ──────────────────────────────────
    // apiKey may be null (read the provider's env var from the registry entry);
    // configJson may be null (defaults) or a JSON object of ProviderOptions.

    long aimux_provider_new(String name, String apiKey, String modelId, String configJson);

    long aimux_provider_from_env(String name, String modelId);

    // ── Generation ──────────────────────────────────────────────────────────

    /** @return JSON result string (caller MUST free with {@link #aimux_free_string}). */
    Pointer aimux_generate_text(long handle, String promptJson, String optsJson);

    /** Push streaming with callbacks; blocks until the stream ends. */
    void aimux_stream_text(long handle, String promptJson, String optsJson,
                           Callback onPart, Callback onDone, Callback onError);

    // ── Resource management ─────────────────────────────────────────────────

    void aimux_drop_handle(long handle);

    void aimux_free_string(Pointer ptr);

    // ── Embedding ───────────────────────────────────────────────────────────

    long aimux_openai_embedding_new(String apiKey, String modelId);

    long aimux_openai_embedding_new_with_base(String apiKey, String modelId, String baseUrl);

    long aimux_cohere_embedding_new(String apiKey, String modelId);

    long aimux_cohere_embedding_new_with_base(String apiKey, String modelId, String baseUrl);

    long aimux_google_embedding_new(String apiKey, String modelId);

    long aimux_google_embedding_new_with_base(String apiKey, String modelId, String baseUrl);

    Pointer aimux_embed(long handle, String valuesJson, String optsJson);

    // ── Speech (TTS) ────────────────────────────────────────────────────────

    long aimux_openai_speech_new(String apiKey, String modelId);

    long aimux_openai_speech_new_with_base(String apiKey, String modelId, String baseUrl);

    Pointer aimux_speech_generate(long handle, String optsJson);

    // ── Image ───────────────────────────────────────────────────────────────

    long aimux_openai_image_new(String apiKey, String modelId);

    long aimux_openai_image_new_with_base(String apiKey, String modelId, String baseUrl);

    long aimux_google_image_new(String apiKey, String modelId);

    long aimux_google_image_new_with_base(String apiKey, String modelId, String baseUrl);

    Pointer aimux_image_generate(long handle, String optsJson);

    // ── Transcription (STT, non-streaming) ──────────────────────────────────

    long aimux_openai_transcription_new(String apiKey, String modelId);

    long aimux_openai_transcription_new_with_base(String apiKey, String modelId, String baseUrl);

    Pointer aimux_transcription_generate(long handle, String audioBase64,
                                         String mediaType, String optsJson);

    // ── Files ───────────────────────────────────────────────────────────────

    long aimux_openai_files_new(String apiKey);

    long aimux_openai_files_new_with_base(String apiKey, String baseUrl);

    Pointer aimux_file_upload(long handle, String dataBase64,
                              String mediaType, String optsJson);

    // ── Reranking ───────────────────────────────────────────────────────────

    long aimux_cohere_reranking_new(String apiKey, String modelId);

    long aimux_cohere_reranking_new_with_base(String apiKey, String modelId, String baseUrl);

    Pointer aimux_rerank(long handle, String optsJson);

    // ── Video ───────────────────────────────────────────────────────────────

    long aimux_google_video_new(String apiKey, String modelId);

    long aimux_google_video_new_with_base(String apiKey, String modelId, String baseUrl);

    Pointer aimux_video_generate(long handle, String optsJson);

    // ── Search ──────────────────────────────────────────────────────────────

    long aimux_tavily_search_new(String apiKey, String modelId);

    long aimux_tavily_search_new_with_base(String apiKey, String modelId, String baseUrl);

    Pointer aimux_search(long handle, String optsJson);
}
