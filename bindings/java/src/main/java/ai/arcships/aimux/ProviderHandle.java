package ai.arcships.aimux;

/**
 * A provider handle — created by {@link Model#createProvider}, supports
 * {@link #listModels()} (runtime discovery) and {@link #model(String)} (build a
 * model from a discovered id).
 *
 * <p>Implements {@link AutoCloseable}; the native handle is released on
 * {@link #close()}. Use try-with-resources or call {@code close()} explicitly.
 */
public class ProviderHandle implements AutoCloseable {

    private long handle;
    private boolean closed;

    ProviderHandle(long handle) {
        this.handle = handle;
    }

    @Override
    public synchronized void close() {
        if (!closed && handle != 0) {
            AimuxFFI.INSTANCE.aimux_drop_handle(handle);
            handle = 0;
            closed = true;
        }
    }

    @Override
    protected void finalize() {
        close();
    }

    /**
     * List models available on this provider (runtime discovery via the
     * provider's {@code /models} endpoint), enriched with community knowledge
     * (anya2a) when available.
     *
     * @return a JSON array of ResolvedModel
     * @throws IllegalStateException if this handle is closed
     * @throws AimuxException on engine / transport failure
     */
    public String listModels() {
        if (closed || handle == 0) {
            throw new IllegalStateException("ProviderHandle is closed");
        }
        AimuxCError err = AimuxResult.newError();
        return AimuxResult.extractString(
            AimuxFFI.INSTANCE.aimux_provider_list_models(handle, err),
            err,
            "list_models");
    }

    /**
     * Build a language model from a discovered model id.
     *
     * @param modelId the model id (e.g. from {@link #listModels()})
     * @return a new {@link Model}
     * @throws IllegalStateException if this handle is closed
     * @throws AimuxException on construction failure
     */
    public Model model(String modelId) {
        if (closed || handle == 0) {
            throw new IllegalStateException("ProviderHandle is closed");
        }
        AimuxCError err = AimuxResult.newError();
        long h = AimuxFFI.INSTANCE.aimux_provider_model(handle, modelId, err);
        return new Model(AimuxResult.extractHandle(h, err, "Failed to create model: " + modelId));
    }
}
