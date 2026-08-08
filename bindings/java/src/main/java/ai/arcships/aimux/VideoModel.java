package ai.arcships.aimux;

import java.io.Closeable;
import java.util.concurrent.atomic.AtomicLong;

/**
 * Generates videos from prompts. Wraps a Rust {@code Arc<dyn VideoModel>}.
 *
 * <p>Implements {@link Closeable} — call {@link #close()} (or use a
 * try-with-resources block) to release the native handle.
 *
 * <pre>{@code
 * try (VideoModel model = VideoModel.google("sk-...", "veo-3.0")) {
 *     String result = model.generate("{\"prompt\":\"a sunset\",\"n\":1}");
 * }
 * }</pre>
 */
public final class VideoModel implements Closeable {

    private final AtomicLong handle;

    private VideoModel(long handle) {
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
        if (h == 0L) throw new IllegalStateException("VideoModel is closed");
        return h;
    }

    /**
     * Create a Google video model.
     *
     * @param apiKey  Google API key.
     * @param modelId Model ID (e.g. {@code veo-3.0}).
     * @return a new VideoModel.
     * @throws AimuxException on failure.
     */
    public static VideoModel google(String apiKey, String modelId) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_google_video_new(apiKey, modelId, err);
        return new VideoModel(AimuxResult.extractHandle(h, err, "Failed to create Google video model"));
    }

    /**
     * Create a Google video model with a custom base URL.
     *
     * @param apiKey  Google API key.
     * @param modelId Model ID.
     * @param baseUrl Custom base URL.
     * @return a new VideoModel.
     * @throws AimuxException on failure.
     */
    public static VideoModel googleWithBase(String apiKey, String modelId, String baseUrl) {
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_google_video_new_with_base(apiKey, modelId, baseUrl, err);
        return new VideoModel(AimuxResult.extractHandle(h, err, "Failed to create Google video model"));
    }

    /**
     * Generate videos from the given options.
     *
     * @param optsJson JSON-serialized {@code VideoCallOptions}.
     * @return JSON-serialized {@code VideoResult}.
     * @throws AimuxException on engine / transport failure.
     */
    public String generate(String optsJson) {
        AimuxCError err = AimuxResult.newError();
        return AimuxResult.extractString(
            AimuxFFI.INSTANCE.aimux_video_generate(requireHandle(), optsJson, err),
            err,
            "video_generate");
    }
}
