package ai.arcships.aimux;

import ai.arcships.aimux.types.*;

/** A short admission lock; no monitor is held across native calls or callbacks. */
abstract class NativeHandle implements AutoCloseable {
    final Aimux aimux;
    private final long id;
    private int active;
    private boolean closed;

    NativeHandle(Aimux aimux, long id) {
        if (id == 0) throw new IllegalArgumentException("Resource handle cannot be zero");
        this.aimux = java.util.Objects.requireNonNull(aimux, "aimux");
        this.id = id;
    }

    public final String getHandle() { return Long.toUnsignedString(id); }

    final synchronized Lease admit() {
        if (closed) throw new IllegalStateException("Resource is closed");
        active++;
        return new Lease(this, id);
    }

    private void release() {
        boolean drop;
        synchronized (this) { drop = --active == 0 && closed; }
        if (drop) aimux.drop(id);
    }

    /** Calls an operation using its manifest request DTO unchanged. */
    public final com.fasterxml.jackson.databind.JsonNode call(String op, String paramsJson) {
        String params = AimuxResult.params(paramsJson);
        try (Lease lease = admit(); Abort abort = aimux.createAbort(); Lease signal = abort.admit()) {
            return aimux.call(lease.id, op, params, signal.id);
        }
    }

    public final <T> T call(String op, WireValue params, Class<T> resultType) {
        return WireJson.decode(call(op, params.toString()), resultType);
    }

    public final com.fasterxml.jackson.databind.JsonNode call(String op, String paramsJson, Abort abort) {
        java.util.Objects.requireNonNull(abort, "abort");
        if (abort.aimux != aimux) throw new IllegalArgumentException("Abort belongs to another library");
        String params = AimuxResult.params(paramsJson);
        try (Lease lease = admit(); Lease signal = abort.admit()) {
            return aimux.call(lease.id, op, params, signal.id);
        }
    }

    public final java.util.concurrent.Future<com.fasterxml.jackson.databind.JsonNode> callAsync(
            String op, String paramsJson, java.util.concurrent.Executor executor) {
        return aimux.submit(this, op, paramsJson, null, executor);
    }

    /** Consumer runs synchronously, providing backpressure. It must return in bounded time. */
    public final void stream(String op, String paramsJson, java.util.function.Consumer<StreamFrame> consumer) {
        String params = AimuxResult.params(paramsJson);
        try (Lease lease = admit(); Abort abort = aimux.createAbort(); Lease signal = abort.admit()) {
            aimux.stream(lease.id, op, params, signal.id, consumer);
        }
    }

    public final void stream(String op, String paramsJson, Abort abort,
                             java.util.function.Consumer<StreamFrame> consumer) {
        java.util.Objects.requireNonNull(abort, "abort");
        if (abort.aimux != aimux) throw new IllegalArgumentException("Abort belongs to another library");
        String params = AimuxResult.params(paramsJson);
        try (Lease lease = admit(); Lease signal = abort.admit()) {
            aimux.stream(lease.id, op, params, signal.id, consumer);
        }
    }

    public final java.util.concurrent.Future<com.fasterxml.jackson.databind.JsonNode> streamAsync(
            String op, String paramsJson, java.util.function.Consumer<StreamFrame> consumer,
            java.util.concurrent.Executor executor) {
        java.util.Objects.requireNonNull(consumer, "consumer");
        return aimux.submit(this, op, paramsJson, consumer, executor);
    }

    /** Transfers ownership of a newly returned child handle. */
    public final Resource create(String op, String paramsJson) {
        return new Resource(aimux, AimuxResult.handle(call(op, paramsJson)));
    }

    /** Stops admission immediately; releases after the last admitted call, without cancelling it. */
    @Override public final void close() {
        boolean drop;
        synchronized (this) {
            if (closed) return;
            closed = true;
            drop = active == 0;
        }
        if (drop) aimux.drop(id);
    }

    static final class Lease implements AutoCloseable {
        private NativeHandle resource;
        final long id;
        Lease(NativeHandle resource, long id) { this.resource = resource; this.id = id; }
        @Override public void close() {
            NativeHandle value;
            synchronized (this) { value = resource; resource = null; }
            if (value != null) value.release();
        }
    }
}
