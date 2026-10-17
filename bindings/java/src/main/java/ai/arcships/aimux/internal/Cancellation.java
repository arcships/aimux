package ai.arcships.aimux.internal;

import java.util.Objects;
import java.util.concurrent.Callable;
import java.util.concurrent.Executor;
import java.util.concurrent.Future;
import java.util.concurrent.FutureTask;
import java.util.concurrent.atomic.AtomicBoolean;

/** Executor-driven calls with cancellation forwarded to the native abort signal. */
public final class Cancellation {
    private Cancellation() {}

    /**
     * Schedules work on the supplied executor. The abort action must be bounded
     * and is called at most once, only when cancellation wins over completion.
     * Executor rejection is reported directly to the caller.
     */
    public static <T> Future<T> submit(Executor executor, Callable<T> work, Runnable abort) {
        Objects.requireNonNull(executor, "executor");
        Objects.requireNonNull(work, "work");
        Objects.requireNonNull(abort, "abort");
        FutureTask<T> task = new AbortableTask<>(work, abort);
        executor.execute(task);
        return task;
    }

    private static final class AbortableTask<T> extends FutureTask<T> {
        private final Runnable abort;
        private final AtomicBoolean aborted = new AtomicBoolean();

        AbortableTask(Callable<T> work, Runnable abort) {
            super(work);
            this.abort = abort;
        }

        @Override
        public boolean cancel(boolean mayInterruptIfRunning) {
            boolean cancelled = super.cancel(mayInterruptIfRunning);
            if (cancelled && aborted.compareAndSet(false, true)) {
                abort.run();
            }
            return cancelled;
        }
    }
}
