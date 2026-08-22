package ai.arcships.aimux;

import com.sun.jna.Pointer;
import com.sun.jna.ptr.LongByReference;
import com.sun.jna.ptr.PointerByReference;
import java.io.Closeable;
import java.util.Objects;
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
     * @return a new SpeechModel.
     * @throws AimuxException if the native constructor fails.
     */
    public static SpeechModel openai(String apiKey, String modelId) {
        Objects.requireNonNull(apiKey, "apiKey");
        Objects.requireNonNull(modelId, "modelId");
        LongByReference out = new LongByReference();
        Pointer e = AimuxFFI.INSTANCE.aimux_openai_speech_new(apiKey, modelId, out);
        return new SpeechModel(AimuxResult.extractHandle(e, out, "Failed to create OpenAI speech model"));
    }

    /**
     * Create an OpenAI speech model with a custom base URL.
     *
     * @param apiKey  OpenAI API key.
     * @param modelId Model ID (e.g. {@code tts-1}).
     * @param baseUrl Custom base URL (e.g. for proxies or local servers).
     * @return a new SpeechModel.
     * @throws AimuxException on failure.
     */
    public static SpeechModel openaiWithBase(String apiKey, String modelId, String baseUrl) {
        Objects.requireNonNull(apiKey, "apiKey");
        Objects.requireNonNull(modelId, "modelId");
        LongByReference out = new LongByReference();
        Pointer e = AimuxFFI.INSTANCE.aimux_openai_speech_new_with_base(apiKey, modelId, baseUrl, out);
        return new SpeechModel(AimuxResult.extractHandle(e, out, "Failed to create OpenAI speech model"));
    }

    /**
     * Generate speech audio from the given options.
     *
     * @param optsJson JSON-serialized {@code SpeechCallOptions}.
     *                 Required: carries the input.
     * @return JSON-serialized {@code SpeechResult}.
     * @throws NullPointerException if {@code optsJson} is null.
     * @throws IllegalArgumentException if {@code optsJson} is blank or malformed JSON.
     * @throws AimuxException on engine / transport failure.
     */
    public String generate(String optsJson) {
        AimuxResult.requireJsonNonNull(optsJson, "optsJson");
        PointerByReference out = new PointerByReference();
        return AimuxResult.extractString(
            AimuxFFI.INSTANCE.aimux_speech_generate(requireHandle(), optsJson, out),
            out,
            "speech_generate");
    }
}
