package ai.arcships.aimux;

import com.sun.jna.Pointer;
import com.sun.jna.ptr.LongByReference;
import com.sun.jna.ptr.PointerByReference;
import java.io.Closeable;
import java.util.Objects;
import java.util.concurrent.atomic.AtomicLong;

/**
 * Reranks documents by relevance to a query. Wraps a Rust
 * {@code Arc<dyn RerankingModel>}.
 *
 * <p>Implements {@link Closeable} — call {@link #close()} (or use a
 * try-with-resources block) to release the native handle.
 *
 * <pre>{@code
 * try (RerankingModel model = RerankingModel.cohere("sk-...", "rerank-v3.0")) {
 *     String result = model.rerank("{\"query\":\"...\",\"documents\":{...}}");
 * }
 * }</pre>
 */
public final class RerankingModel implements Closeable {

    private final AtomicLong handle;

    private RerankingModel(long handle) {
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
        if (h == 0L) throw new IllegalStateException("RerankingModel is closed");
        return h;
    }

    /**
     * Create a Cohere reranking model.
     *
     * @param apiKey  Cohere API key.
     * @param modelId Model ID (e.g. {@code rerank-v3.0}).
     * @return a new RerankingModel.
     * @throws AimuxException on failure.
     */
    public static RerankingModel cohere(String apiKey, String modelId) {
        Objects.requireNonNull(apiKey, "apiKey");
        Objects.requireNonNull(modelId, "modelId");
        LongByReference out = new LongByReference();
        Pointer e = AimuxFFI.INSTANCE.aimux_cohere_reranking_new(apiKey, modelId, out);
        return new RerankingModel(
            AimuxResult.extractHandle(e, out, "Failed to create Cohere reranking model"));
    }

    /**
     * Create a Cohere reranking model with a custom base URL.
     *
     * @param apiKey  Cohere API key.
     * @param modelId Model ID.
     * @param baseUrl Custom base URL.
     * @return a new RerankingModel.
     * @throws AimuxException on failure.
     */
    public static RerankingModel cohereWithBase(String apiKey, String modelId, String baseUrl) {
        Objects.requireNonNull(apiKey, "apiKey");
        Objects.requireNonNull(modelId, "modelId");
        LongByReference out = new LongByReference();
        Pointer e = AimuxFFI.INSTANCE.aimux_cohere_reranking_new_with_base(apiKey, modelId, baseUrl, out);
        return new RerankingModel(
            AimuxResult.extractHandle(e, out, "Failed to create Cohere reranking model"));
    }

    /**
     * Rerank documents against a query.
     *
     * @param optsJson JSON-serialized {@code RerankingCallOptions}.
     *                 Required: carries the input.
     * @return JSON-serialized {@code RerankingResult}.
     * @throws NullPointerException if {@code optsJson} is null.
     * @throws IllegalArgumentException if {@code optsJson} is blank or malformed JSON.
     * @throws AimuxException on engine / transport failure.
     */
    public String rerank(String optsJson) {
        AimuxResult.requireJsonNonNull(optsJson, "optsJson");
        PointerByReference out = new PointerByReference();
        return AimuxResult.extractString(
            AimuxFFI.INSTANCE.aimux_rerank(requireHandle(), optsJson, out),
            out,
            "rerank");
    }
}
