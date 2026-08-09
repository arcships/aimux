package ai.arcships.aimux;

import java.util.concurrent.locks.ReentrantReadWriteLock;

/**
 * A provider handle — created by {@link Model#createProvider}, supports
 * {@link #listModels()} (runtime discovery) and {@link #model(String)} (build a
 * model from a discovered id).
 *
 * <p>Implements {@link AutoCloseable}; the native handle is released on
 * {@link #close()}. Use try-with-resources or call {@code close()} explicitly.
 *
 * <p><b>Thread-safety / concurrency.</b> Safe for concurrent use. The native
 * handle is guarded by a Go-style {@link ReentrantReadWriteLock} (fair, FIFO):
 * {@link #listModels()} and {@link #model(String)} hold the <em>read</em> lock
 * for the entire FFI call, and {@link #close()} takes the <em>write</em> lock.
 * {@code close()} therefore blocks until in-flight calls finish before dropping
 * the native handle — closes the check-then-use use-after-free race where a
 * caller could pass the closed-flag check and then race with {@code close()}'s
 * drop.
 */
public class ProviderHandle implements AutoCloseable {

    // Go-style read/write lock — see Model for the rationale. Each FFI call
    // (listModels/model) holds the read lock for its whole duration; close()
    // takes the write lock and waits for in-flight calls before dropping the
    // handle, preventing a use-after-free.
    private final ReentrantReadWriteLock lock = new ReentrantReadWriteLock(true);
    private long handle;
    private boolean closed;

    ProviderHandle(long handle) {
        this.handle = handle;
    }

    /**
     * Release the native handle. Idempotent and thread-safe: subsequent calls
     * are no-ops. Acquires the write lock and blocks until in-flight
     * {@link #listModels()} / {@link #model(String)} calls finish before
     * dropping the native handle (prevents use-after-free).
     */
    @Override
    public void close() {
        lock.writeLock().lock();
        try {
            if (closed || handle == 0) {
                return;
            }
            long h = handle;
            handle = 0;
            closed = true;
            AimuxFFI.INSTANCE.aimux_drop_handle(h);
        } finally {
            lock.writeLock().unlock();
        }
    }

    @Override
    protected void finalize() {
        close();
    }

    // Caller MUST already hold the read lock; held across the FFI call so
    // close() cannot drop the handle mid-call.
    private long requireHandleLocked() {
        if (closed || handle == 0) {
            throw new IllegalStateException("ProviderHandle is closed");
        }
        return handle;
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
        lock.readLock().lock();
        try {
            long h = requireHandleLocked();
            AimuxCError err = AimuxResult.newError();
            return AimuxResult.extractString(
                AimuxFFI.INSTANCE.aimux_provider_list_models(h, err),
                err,
                "list_models");
        } finally {
            lock.readLock().unlock();
        }
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
        lock.readLock().lock();
        try {
            long h = requireHandleLocked();
            AimuxCError err = AimuxResult.newError();
            long newHandle = AimuxFFI.INSTANCE.aimux_provider_model(h, modelId, err);
            return new Model(AimuxResult.extractHandle(newHandle, err, "Failed to create model: " + modelId));
        } finally {
            lock.readLock().unlock();
        }
    }
}
