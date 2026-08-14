package ai.arcships.aimux;

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
     * @return a new TranscriptionModel.
     * @throws AimuxException on failure.
     */
    public static TranscriptionModel openai(String apiKey, String modelId) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_openai_transcription_new(apiKey, modelId, err);
        return new TranscriptionModel(
            AimuxResult.extractHandle(h, err, "Failed to create OpenAI transcription model"));
    }

    /**
     * Create an OpenAI transcription model with a custom base URL.
     *
     * @param apiKey  OpenAI API key.
     * @param modelId Model ID (e.g. {@code whisper-1}).
     * @param baseUrl Custom base URL.
     * @return a new TranscriptionModel.
     * @throws AimuxException on failure.
     */
    public static TranscriptionModel openaiWithBase(String apiKey, String modelId, String baseUrl) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_openai_transcription_new_with_base(apiKey, modelId, baseUrl, err);
        return new TranscriptionModel(
            AimuxResult.extractHandle(h, err, "Failed to create OpenAI transcription model"));
    }

    /**
     * Transcribe audio (base64-encoded) to text.
     *
     * @param audioBase64 Base64-encoded audio bytes.
     * @param mediaType   Media type of the audio (e.g. {@code audio/wav}).
     * @return JSON-serialized {@code TranscriptionResult}.
     * @throws AimuxException on engine / transport failure.
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
     * @return JSON-serialized {@code TranscriptionResult}.
     * @throws AimuxException on engine / transport failure.
     */
    /**
     * Start a streaming transcription session (RFC-0028) on this model.
     * Requires a model that supports streaming (realtime models).
     *
     * @param optsJson optional session options JSON
     *                 ({@code input_audio_format} / {@code provider_options}
     *                 / {@code headers} / {@code include_raw_chunks}), or
     *                 {@code null} for defaults.
     * @return a new live session.
     * @throws AimuxException on failure.
     */
    public TranscriptionSession startStream(String optsJson) {
        return startStream(optsJson, 0L);
    }

    /**
     * Start a streaming transcription session with an abort handle
     * (from {@code Aimux.abortSignalNew()}); firing it aborts the session.
     *
     * @param optsJson    optional session options JSON, or {@code null}.
     * @param abortHandle abort handle, or 0 for none.
     * @return a new live session.
     * @throws AimuxException on failure.
     */
    public TranscriptionSession startStream(String optsJson, long abortHandle) {
        long h = requireHandle();
        AimuxCError err = AimuxResult.newError();
        long session = AimuxFFI.INSTANCE.aimux_transcription_session_new(h, abortHandle, optsJson, err);
        return new TranscriptionSession(
                AimuxResult.extractHandle(session, err, "Failed to start transcription session"));
    }

    /**
     * Transcribe audio (base64-encoded) to text.
     *
     * @param audioBase64 Base64-encoded audio bytes.
     * @param mediaType   Media type of the audio (e.g. {@code audio/wav}).
     * @param optsJson    Optional JSON-serialized {@code TranscriptionCallOptions},
     *                    or {@code null} for defaults.
     * @return JSON-serialized {@code TranscriptionResult}.
     * @throws AimuxException on engine / transport failure.
     */
    public String generate(String audioBase64, String mediaType, String optsJson) {
        AimuxCError err = AimuxResult.newError();
        return AimuxResult.extractString(
            AimuxFFI.INSTANCE.aimux_transcription_generate(
                requireHandle(), audioBase64, mediaType, optsJson, err),
            err,
            "transcription_generate");
    }
}
