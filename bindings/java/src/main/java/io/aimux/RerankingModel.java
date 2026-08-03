package io.aimux;

import java.io.Closeable;
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
     * @return a new RerankingModel; throws {@link IllegalArgumentException} on failure.
     */
    public static RerankingModel cohere(String apiKey, String modelId) {
        long h = AimuxResult.extractHandle(
            AimuxFFI.INSTANCE.aimux_cohere_reranking_new(apiKey, modelId), "Failed to create Cohere reranking model");
        return new RerankingModel(h);
    }

    /**
     * Create a Cohere reranking model with a custom base URL.
     *
     * @param apiKey  Cohere API key.
     * @param modelId Model ID.
     * @param baseUrl Custom base URL.
     * @return a new RerankingModel; throws {@link IllegalArgumentException} on failure.
     */
    public static RerankingModel cohereWithBase(String apiKey, String modelId, String baseUrl) {
        long h = AimuxResult.extractHandle(
            AimuxFFI.INSTANCE.aimux_cohere_reranking_new_with_base(apiKey, modelId, baseUrl),
            "Failed to create Cohere reranking model");
        return new RerankingModel(h);
    }

    /**
     * Rerank documents against a query.
     *
     * @param optsJson JSON-serialized {@code RerankingCallOptions}.
     * @return JSON-serialized {@code RerankingResult}. If the engine returns
     *         an {@code {"error":"..."}} envelope, an {@link AimuxException} is thrown.
     */
    public String rerank(String optsJson) {
        return AimuxResult.extractString(
            AimuxFFI.INSTANCE.aimux_rerank(requireHandle(), optsJson), "rerank");
    }
}
