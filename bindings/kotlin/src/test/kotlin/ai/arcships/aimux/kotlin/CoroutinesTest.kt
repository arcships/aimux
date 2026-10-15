package ai.arcships.aimux.kotlin

import ai.arcships.aimux.AimuxException as JavaAimuxException
import ai.arcships.aimux.KotlinInteropFixtures
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.launch
import kotlinx.coroutines.newSingleThreadContext
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test
import java.util.concurrent.CompletableFuture
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference

class CoroutinesTest {
    private fun javaError(name: String, code: Int): JavaAimuxException =
        KotlinInteropFixtures.decode(
            """{"kind":"aimux","code":$code,"message":"failure","error":{"name":"$name"}}"""
        ) as JavaAimuxException

    private class Operation : CompletableFuture<Unit>() {
        val aborts = AtomicInteger()
        override fun cancel(mayInterruptIfRunning: Boolean): Boolean {
            assertTrue(mayInterruptIfRunning)
            aborts.incrementAndGet()
            return super.cancel(mayInterruptIfRunning)
        }
    }

    @Test
    fun `successful Flow drains in order without cancellation`() = runBlocking {
        val operation = Operation()
        val values = aimuxFlow<Int>(2) { executor, consumer ->
            executor.execute {
                repeat(20) { consumer.accept(it) }
                operation.complete(Unit)
            }
            operation
        }.toList()
        assertEquals((0 until 20).toList(), values)
        assertEquals(0, operation.aborts.get())
    }

    @Test
    fun `take one cancels Future and releases full callback queue`() = runBlocking {
        val operation = Operation()
        val exited = CountDownLatch(1)
        val values = aimuxFlow<Int>(1) { executor, consumer ->
            executor.execute {
                try {
                    for (value in 0 until 1000) {
                        if (operation.isCancelled) break
                        consumer.accept(value)
                    }
                } finally {
                    exited.countDown()
                }
            }
            operation
        }.take(1).toList()
        assertEquals(listOf(0), values)
        assertEquals(1, operation.aborts.get())
        assertTrue(exited.await(2, TimeUnit.SECONDS))
    }

    @Test
    fun `cancelled Future retains callback lifetime until executor task returns`() = runBlocking {
        val cancelled = CompletableDeferred<Unit>()
        val release = CountDownLatch(1)
        val operation = object : CompletableFuture<Unit>() {
            override fun cancel(mayInterruptIfRunning: Boolean): Boolean {
                val changed = super.cancel(mayInterruptIfRunning)
                cancelled.complete(Unit)
                return changed
            }
        }
        val collection = async {
            aimuxFlow<Int>(1) { executor, consumer ->
                executor.execute {
                    consumer.accept(1)
                    assertTrue(release.await(5, TimeUnit.SECONDS))
                }
                operation
            }.take(1).toList()
        }
        try {
            withTimeout(2000) { cancelled.await() }
            assertTrue(operation.isDone)
            assertFalse(collection.isCompleted)
        } finally { release.countDown() }
        assertEquals(listOf(1), withTimeout(2000) { collection.await() })
    }

    @Test
    fun `slow collector imposes a finite producer bound`() = runBlocking {
        val operation = Operation()
        val entered = CompletableDeferred<Unit>()
        val release = CompletableDeferred<Unit>()
        val produced = AtomicInteger()
        val exited = CountDownLatch(1)
        val job = launch {
            aimuxFlow<Int>(2) { executor, consumer ->
                executor.execute {
                    try {
                        for (value in 0 until 1000) {
                            if (operation.isCancelled) break
                            consumer.accept(value)
                            produced.incrementAndGet()
                        }
                    } finally {
                        exited.countDown()
                    }
                }
                operation
            }.collect {
                entered.complete(Unit)
                release.await()
            }
        }
        withTimeout(2000) { entered.await() }
        delay(100)
        // Two queued values and the collector's value; no extra dispatcher buffer.
        assertTrue(produced.get() in 1..3, "produced=${produced.get()}")
        job.cancelAndJoin()
        assertEquals(1, operation.aborts.get())
        assertTrue(exited.await(2, TimeUnit.SECONDS))
    }

    @Test
    fun `explicit cancellation before first event triggers Abort`() = runBlocking {
        val operation = Operation()
        val started = CompletableDeferred<Unit>()
        val job = launch {
            aimuxFlow<Int> { _, _ ->
                started.complete(Unit)
                operation
            }.collect()
        }
        withTimeout(2000) { started.await() }
        job.cancelAndJoin()
        assertEquals(1, operation.aborts.get())
    }

    @Test
    fun `collector exception cancels producer and is preserved`() = runBlocking {
        val operation = Operation()
        val expected = IllegalStateException("collector")
        try {
            aimuxFlow<Int> { executor, consumer ->
                executor.execute { consumer.accept(1) }
                operation
            }.collect { throw expected }
            fail("expected collector exception")
        } catch (error: IllegalStateException) {
            assertSame(expected, generateSequence<Throwable>(error) { it.cause }.last())
        }
        assertEquals(1, operation.aborts.get())
    }

    @Test
    fun `Java Future failure follows buffered events as sealed error`() = runBlocking {
        val javaError = javaError("AI_APICallError", 11)
        val values = mutableListOf<Int>()
        try {
            aimuxFlow<Int>(2) { executor, consumer ->
                val future = CompletableFuture<Unit>()
                executor.execute {
                    consumer.accept(1)
                    consumer.accept(2)
                    future.completeExceptionally(javaError)
                }
                future
            }.collect { values.add(it) }
            fail("expected error")
        } catch (error: APICallError) {
            assertSame(javaError, error.javaError)
        }
        assertEquals(listOf(1, 2), values)
    }

    @Test
    fun `startup exception maps without waiting for an event`() = runBlocking {
        val javaError = javaError("AI_InvalidPromptError", 6)
        try {
            aimuxFlow<Int> { _, _ -> throw javaError }.collect()
            fail("expected startup failure")
        } catch (error: InvalidPromptError) {
            assertSame(javaError, error.javaError)
        }
    }

    @Test
    fun `Flow is cold and each collection starts its own operation`() = runBlocking {
        val starts = AtomicInteger()
        val stream = aimuxFlow<Int> { _, consumer ->
            consumer.accept(starts.incrementAndGet())
            CompletableFuture.completedFuture(Unit)
        }
        assertEquals(0, starts.get())
        assertEquals(listOf(1), stream.toList())
        assertEquals(listOf(2), stream.toList())
    }

    @Test
    fun `an error value in the event payload is ordinary stream data`() = runBlocking {
        val event = javaError("AI_APICallError", 11)
        val values = aimuxFlow<JavaAimuxException> { _, consumer ->
            consumer.accept(event)
            CompletableFuture.completedFuture(Unit)
        }.toList()
        assertSame(event, values.single())
    }

    @Test
    fun `await cancellation invokes the Java cancellation hook`() = runBlocking {
        val operation = Operation()
        val started = CompletableDeferred<Unit>()
        val job = launch {
            aimuxCall {
                started.complete(Unit)
                operation
            }
        }
        withTimeout(2000) { started.await() }
        job.cancelAndJoin()
        assertEquals(1, operation.aborts.get())
    }

    @Test
    fun `cancellation during admission cancels Future once it returns`() = runBlocking {
        val operation = Operation()
        val entered = CompletableDeferred<Unit>()
        val release = CountDownLatch(1)
        val job = launch {
            aimuxCall {
                entered.complete(Unit)
                assertTrue(release.await(2, TimeUnit.SECONDS))
                operation
            }
        }
        withTimeout(2000) { entered.await() }
        job.cancel()
        release.countDown()
        job.join()
        assertEquals(1, operation.aborts.get())
    }

    @Test
    fun `cancellation during Flow admission cancels the returned Future`() = runBlocking {
        val operation = Operation()
        val entered = CompletableDeferred<Unit>()
        val release = CountDownLatch(1)
        val job = launch {
            aimuxFlow<Int> { _, _ ->
                entered.complete(Unit)
                assertTrue(release.await(2, TimeUnit.SECONDS))
                operation
            }.collect()
        }
        withTimeout(2000) { entered.await() }
        job.cancel()
        release.countDown()
        job.join()
        assertEquals(1, operation.aborts.get())
    }

    @Test
    fun `Java cancellation keeps its source error through call and Flow`() = runBlocking {
        val source = javaError("AbortError", 13)
        val cancellation = CancellationException("Native stream cancelled").apply { initCause(source) }
        val future = CompletableFuture<Unit>().apply { completeExceptionally(cancellation) }
        for (await in listOf<suspend () -> Unit>(
            { aimuxCall { future } },
            { aimuxFlow<Int> { _, _ -> future }.collect() },
        )) {
            try {
                await()
                fail("expected cancellation")
            } catch (error: CancellationException) {
                assertEquals(cancellation.message, error.message)
                assertSame(source, generateSequence<Throwable>(error) { it.cause }.last())
            }
        }
    }

    @Test
    fun `await maps Java failure and unwraps CompletionException`() = runBlocking {
        val javaError = javaError("TimeoutError", 12)
        val future = CompletableFuture<Unit>()
        future.completeExceptionally(java.util.concurrent.CompletionException(javaError))
        try {
            future.awaitAimux()
            fail("expected error")
        } catch (error: TimeoutError) {
            assertSame(javaError, error.javaError)
        }
    }

    @OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class, kotlinx.coroutines.DelicateCoroutinesApi::class)
    @Test
    fun `blocking startup and Future wait leave caller UI dispatcher free`() = runBlocking {
        newSingleThreadContext("test-ui").use { ui ->
            withContext(ui) {
                val caller = Thread.currentThread()
                val operation = Operation()
                val thread = AtomicReference<Thread>()
                val started = CompletableDeferred<Unit>()
                val call = async {
                    aimuxCall {
                        thread.set(Thread.currentThread())
                        started.complete(Unit)
                        Thread.sleep(100)
                        operation
                    }
                }
                withTimeout(2000) { started.await() }
                assertNotSame(caller, thread.get())
                assertSame(caller, Thread.currentThread())
                operation.complete(Unit)
                withTimeout(2000) { call.await() }
            }
        }
    }

    @Test
    fun `invalid bounds reject immediately`() {
        for (capacity in listOf(-1, 0, 257, Int.MAX_VALUE)) {
            assertThrows(IllegalArgumentException::class.java) {
                aimuxFlow<Int>(capacity) { _, _ -> CompletableFuture.completedFuture(Unit) }
            }
        }
    }
}
