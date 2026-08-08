package ai.arcships.aimux;

import java.io.Closeable;
import java.util.concurrent.atomic.AtomicLong;

/**
 * Generates images from prompts. Wraps a Rust {@code Arc<dyn ImageModel>}.
 *
 * <p>Implements {@link Closeable} — call {@link #close()} (or use a
 * try-with-resources block) to release the native handle.
 *
 * <pre>{@code
 * try (ImageModel model = ImageModel.openai("sk-...", "dall-e-3")) {
 *     String result = model.generate("{\"prompt\":\"an otter\",\"n\":1}");
 * }
 * }</pre>
 */
public final class ImageModel implements Closeable {

    private final AtomicLong handle;

    private ImageModel(long handle) {
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
        if (h == 0L) throw new IllegalStateException("ImageModel is closed");
        return h;
    }

    /**
     * Create an OpenAI image model.
     *
     * @param apiKey  OpenAI API key.
     * @param modelId Model ID (e.g. {@code dall-e-3}).
     * @return a new ImageModel.
     * @throws AimuxException on failure.
     */
    public static ImageModel openai(String apiKey, String modelId) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_openai_image_new(apiKey, modelId, err);
        return new ImageModel(AimuxResult.extractHandle(h, err, "Failed to create OpenAI image model"));
    }

    /**
     * Create an OpenAI image model with a custom base URL.
     *
     * @param apiKey  OpenAI API key.
     * @param modelId Model ID (e.g. {@code dall-e-3}).
     * @param baseUrl Custom base URL.
     * @return a new ImageModel.
     * @throws AimuxException on failure.
     */
    public static ImageModel openaiWithBase(String apiKey, String modelId, String baseUrl) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_openai_image_new_with_base(apiKey, modelId, baseUrl, err);
        return new ImageModel(AimuxResult.extractHandle(h, err, "Failed to create OpenAI image model"));
    }

    /**
     * Create a Google image model.
     *
     * @param apiKey  Google API key.
     * @param modelId Model ID (e.g. {@code gemini-2.5-flash-image}).
     * @return a new ImageModel.
     * @throws AimuxException on failure.
     */
    public static ImageModel google(String apiKey, String modelId) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_google_image_new(apiKey, modelId, err);
        return new ImageModel(AimuxResult.extractHandle(h, err, "Failed to create Google image model"));
    }

    /**
     * Create a Google image model with a custom base URL.
     *
     * @param apiKey  Google API key.
     * @param modelId Model ID.
     * @param baseUrl Custom base URL.
     * @return a new ImageModel.
     * @throws AimuxException on failure.
     */
    public static ImageModel googleWithBase(String apiKey, String modelId, String baseUrl) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_google_image_new_with_base(apiKey, modelId, baseUrl, err);
        return new ImageModel(AimuxResult.extractHandle(h, err, "Failed to create Google image model"));
    }

    /**
     * Generate images from the given options.
     *
     * @param optsJson JSON-serialized {@code ImageCallOptions}.
     * @return JSON-serialized {@code ImageResult}.
     * @throws AimuxException on engine / transport failure.
     */
    public String generate(String optsJson) {
        AimuxCError err = AimuxResult.newError();
        return AimuxResult.extractString(
            AimuxFFI.INSTANCE.aimux_image_generate(requireHandle(), optsJson, err),
            err,
            "image_generate");
    }
}
