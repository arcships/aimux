package io.aimux;

import java.io.Closeable;
import java.util.concurrent.atomic.AtomicLong;

/**
 * Converts text to speech audio. Wraps a Rust {@code Arc<dyn SpeechModel>}.
 *
 * <p>Implements {@link Closeable} — call {@link #close()} (or use a
 * try-with-resources block) to release the native handle.
 *
 * <pre>{@code
 * try (SpeechModel model = SpeechModel.openai("sk-...", "tts-1")) {
 *     String result = model.generate("{\"text\":\"Hello\",\"voice\":\"alloy\"}");
 * }
 * }</pre>
 */
public final class SpeechModel implements Closeable {

    private final AtomicLong handle;

    private SpeechModel(long handle) {
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
        if (h == 0L) throw new IllegalStateException("SpeechModel is closed");
        return h;
    }

    /**
     * Create an OpenAI speech (TTS) model.
     *
     * @param apiKey  OpenAI API key.
     * @param modelId Model ID (e.g. {@code tts-1}).
     * @return a new SpeechModel; throws {@link IllegalArgumentException} if the
     *         native constructor fails (handle == 0).
     */
    public static SpeechModel openai(String apiKey, String modelId) {
        long h = AimuxFFI.INSTANCE.aimux_openai_speech_new(apiKey, modelId);
        if (h == 0L) throw new IllegalArgumentException("Failed to create OpenAI speech model");
        return new SpeechModel(h);
    }

    /**
     * Create an OpenAI speech model with a custom base URL.
     *
     * @param apiKey  OpenAI API key.
     * @param modelId Model ID (e.g. {@code tts-1}).
     * @param baseUrl Custom base URL (e.g. for proxies or local servers).
     * @return a new SpeechModel; throws {@link IllegalArgumentException} on failure.
     */
    public static SpeechModel openaiWithBase(String apiKey, String modelId, String baseUrl) {
        long h = AimuxFFI.INSTANCE.aimux_openai_speech_new_with_base(apiKey, modelId, baseUrl);
        if (h == 0L) throw new IllegalArgumentException("Failed to create OpenAI speech model");
        return new SpeechModel(h);
    }

    /**
     * Generate speech audio from the given options.
     *
     * @param optsJson JSON-serialized {@code SpeechCallOptions}.
     * @return JSON-serialized {@code SpeechResult}. If the engine returns an
     *         {@code {"error":"..."}} envelope, an {@link AimuxException} is thrown.
     */
    public String generate(String optsJson) {
        return AimuxResult.extractString(
            AimuxFFI.INSTANCE.aimux_speech_generate(requireHandle(), optsJson), "speech_generate");
    }
}
