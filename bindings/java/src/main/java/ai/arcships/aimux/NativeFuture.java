package ai.arcships.aimux;

import ai.arcships.aimux.types.*;

import ai.arcships.aimux.internal.Cancellation;
import com.fasterxml.jackson.databind.JsonNode;
import java.util.concurrent.Callable;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.Executor;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;
import java.util.function.Consumer;

/** Native execution retains its admissions until the executor task returns. */
final class NativeFuture implements Future<JsonNode> {
    private final Future<JsonNode> task;

    NativeFuture(Aimux aimux, NativeHandle.Lease lease, Abort abort, String op,
                 String params, Consumer<StreamFrame> consumer, Executor executor) {
        Work work = new Work(aimux, lease, abort, op, params, consumer);
        try {
            task = Cancellation.submit(command -> executor.execute(() -> work.run(command)), work, work::cancel);
        } catch (RuntimeException | Error error) {
            work.cancel();
            throw error;
        }
    }

    // Thread interruption cannot stop a synchronous native call; always signal native Abort.
    @Override public boolean cancel(boolean mayInterrupt) { return task.cancel(false); }
    @Override public boolean isCancelled() { return task.isCancelled(); }
    @Override public boolean isDone() { return task.isDone(); }
    @Override public JsonNode get() throws InterruptedException, ExecutionException { return task.get(); }
    @Override public JsonNode get(long timeout, TimeUnit unit)
            throws InterruptedException, ExecutionException, TimeoutException {
        return task.get(timeout, unit);
    }

    private static final class Work implements Callable<JsonNode> {
        private final Aimux aimux;
        private final NativeHandle.Lease lease;
        private final Abort abort;
        private final String op;
        private final String params;
        private final Consumer<StreamFrame> consumer;
        private boolean started;
        private boolean cleaned;

        Work(Aimux aimux, NativeHandle.Lease lease, Abort abort, String op, String params,
             Consumer<StreamFrame> consumer) {
            this.aimux = aimux; this.lease = lease; this.abort = abort;
            this.op = op; this.params = params; this.consumer = consumer;
        }

        void run(Runnable command) {
            synchronized (this) {
                if (cleaned || started) return;
                started = true;
            }
            try { command.run(); }
            finally { cleanup(); }
        }

        synchronized void cancel() {
            // Completion can release the admissions between Future.cancel and this action.
            if (cleaned) return;
            try { abort.signal(); }
            finally { if (!started) cleanup(); }
        }

        private synchronized void cleanup() {
            if (cleaned) return;
            cleaned = true;
            try { abort.close(); }
            finally { if (lease != null) lease.close(); }
        }

        @Override public JsonNode call() {
            try (NativeHandle.Lease signal = abort.admit()) {
                long handle = lease == null ? 0 : lease.id;
                if (consumer == null) return aimux.call(handle, op, params, signal.id);
                aimux.stream(handle, op, params, signal.id, consumer);
                return WireJson.MAPPER.nullNode();
            }
        }
    }
}
