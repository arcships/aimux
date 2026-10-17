package ai.arcships.aimux.internal;

import java.util.concurrent.CancellationException;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.RejectedExecutionException;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.*;

class CancellationTest {
    @Test
    void queuedCancellationAbortsOnceAndSkipsWork() {
        AtomicReference<Runnable> queued = new AtomicReference<>();
        AtomicInteger calls = new AtomicInteger();
        AtomicInteger aborts = new AtomicInteger();
        Future<Integer> task = Cancellation.submit(queued::set, calls::incrementAndGet, aborts::incrementAndGet);
        assertTrue(task.cancel(false));
        assertFalse(task.cancel(true));
        queued.get().run();
        assertEquals(0, calls.get());
        assertEquals(1, aborts.get());
        assertThrows(CancellationException.class, task::get);
    }

    @Test
    void completionAndFailureCannotBeRewrittenByLateCancellation() throws Exception {
        AtomicInteger aborts = new AtomicInteger();
        Future<Integer> success = Cancellation.submit(Runnable::run, () -> 7, aborts::incrementAndGet);
        assertEquals(7, success.get().intValue());
        assertFalse(success.cancel(true));
        IllegalStateException failure = new IllegalStateException("native failure");
        Future<Integer> failed = Cancellation.submit(Runnable::run, () -> { throw failure; }, aborts::incrementAndGet);
        assertSame(failure, assertThrows(ExecutionException.class, failed::get).getCause());
        assertFalse(failed.cancel(false));
        assertEquals(0, aborts.get());
    }

    @Test
    void runningCancellationSignalsAbortWithoutRequiringInterruption() throws Exception {
        ExecutorService executor = Executors.newSingleThreadExecutor();
        CountDownLatch started = new CountDownLatch(1);
        CountDownLatch aborted = new CountDownLatch(1);
        CountDownLatch returned = new CountDownLatch(1);
        AtomicInteger aborts = new AtomicInteger();
        try {
            Future<Integer> task = Cancellation.submit(executor, () -> {
                started.countDown();
                try {
                    assertTrue(aborted.await(5, TimeUnit.SECONDS));
                    return 1;
                } finally {
                    returned.countDown();
                }
            }, () -> { aborts.incrementAndGet(); aborted.countDown(); });
            assertTrue(started.await(5, TimeUnit.SECONDS));
            ExecutorService cancellers = Executors.newFixedThreadPool(4);
            try {
                for (int i = 0; i < 16; i++) cancellers.submit(() -> task.cancel(false));
            } finally {
                cancellers.shutdown();
                assertTrue(cancellers.awaitTermination(5, TimeUnit.SECONDS));
            }
            assertTrue(returned.await(5, TimeUnit.SECONDS));
            assertEquals(1, aborts.get());
            assertTrue(task.isCancelled());
        } finally {
            aborted.countDown();
            executor.shutdownNow();
            assertTrue(executor.awaitTermination(5, TimeUnit.SECONDS));
        }
    }

    @Test
    void executorRejectionIsSynchronous() {
        AtomicInteger aborts = new AtomicInteger();
        assertThrows(RejectedExecutionException.class, () -> Cancellation.submit(
            task -> { throw new RejectedExecutionException(); }, () -> 1, aborts::incrementAndGet));
        assertEquals(0, aborts.get());
    }

    @Test
    void throwingAbortIsNotRetried() {
        AtomicInteger aborts = new AtomicInteger();
        Future<Integer> task = Cancellation.submit(ignored -> {}, () -> 1, () -> {
            aborts.incrementAndGet();
            throw new IllegalStateException("abort failure");
        });
        assertThrows(IllegalStateException.class, () -> task.cancel(false));
        assertTrue(task.isCancelled());
        assertFalse(task.cancel(false));
        assertEquals(1, aborts.get());
    }
}
