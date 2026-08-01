package io.aimux;

import java.io.Closeable;
import java.util.concurrent.atomic.AtomicLong;

/**
 * Converts audio to text. Wraps a Rust {@code Arc<dyn TranscriptionModel>}.
 *
 * <p>Implements {@link Closeable} — call {@link #close()} (or use a
 * try-with-resources block) to release the native handle.
 *
 * <pre>{@code
 * try (TranscriptionModel model = TranscriptionModel.openai("sk-...", "whisper-1")) {
 *     String result = model.generate(base64Audio, "audio/wav");
 * }
 * }</pre>
 */
public final class TranscriptionModel implements Closeable {

    private final AtomicLong handle;

    private TranscriptionModel(long handle) {
        this.handle = new AtomicLong(handle);
    }

    /** Release the native handle. Idempotent and thread-safe. */
    @Override
    public void close() {
        long h = handle.getAndSet(0L);
        if (h != 0L) AimuxFFI.INSTANCE.aimux_drop_handle(h);
    }

    /** Best-effort backstop; try-with-resources is the primary release path. */
    @Override
    protected void finalize() throws Throwable {
        close();
        super.finalize();
    }

    private long requireHandle() {
        long h = handle.get();
        if (h == 0L) throw new IllegalStateException("TranscriptionModel is closed");
        return h;
    }

    /**
     * Create an OpenAI transcription (STT) model.
     *
     * @param apiKey  OpenAI API key.
     * @param modelId Model ID (e.g. {@code whisper-1}).
     * @return a new TranscriptionModel; throws {@link IllegalArgumentException} on failure.
     */
    public static TranscriptionModel openai(String apiKey, String modelId) {
        long h = AimuxFFI.INSTANCE.aimux_openai_transcription_new(apiKey, modelId);
        if (h == 0L) throw new IllegalArgumentException("Failed to create OpenAI transcription model");
        return new TranscriptionModel(h);
    }

    /**
     * Create an OpenAI transcription model with a custom base URL.
     *
     * @param apiKey  OpenAI API key.
     * @param modelId Model ID (e.g. {@code whisper-1}).
     * @param baseUrl Custom base URL.
     * @return a new TranscriptionModel; throws {@link IllegalArgumentException} on failure.
     */
    public static TranscriptionModel openaiWithBase(String apiKey, String modelId, String baseUrl) {
        long h = AimuxFFI.INSTANCE.aimux_openai_transcription_new_with_base(apiKey, modelId, baseUrl);
        if (h == 0L) throw new IllegalArgumentException("Failed to create OpenAI transcription model");
        return new TranscriptionModel(h);
    }

    /**
     * Transcribe audio (base64-encoded) to text.
     *
     * @param audioBase64 Base64-encoded audio bytes.
     * @param mediaType   Media type of the audio (e.g. {@code audio/wav}).
     * @return JSON-serialized {@code TranscriptionResult}. If the engine returns
     *         an {@code {"error":"..."}} envelope, an {@link AimuxException} is thrown.
     */
    public String generate(String audioBase64, String mediaType) {
        return generate(audioBase64, mediaType, null);
    }

    /**
     * Transcribe audio (base64-encoded) to text.
     *
     * @param audioBase64 Base64-encoded audio bytes.
     * @param mediaType   Media type of the audio (e.g. {@code audio/wav}).
     * @param optsJson    Optional JSON-serialized {@code TranscriptionCallOptions},
     *                    or {@code null} for defaults.
     * @return JSON-serialized {@code TranscriptionResult}. If the engine returns
     *         an {@code {"error":"..."}} envelope, an {@link AimuxException} is thrown.
     */
    public String generate(String audioBase64, String mediaType, String optsJson) {
        return AimuxResult.extractString(
            AimuxFFI.INSTANCE.aimux_transcription_generate(
                requireHandle(), audioBase64, mediaType, optsJson),
            "transcription_generate");
    }
}
