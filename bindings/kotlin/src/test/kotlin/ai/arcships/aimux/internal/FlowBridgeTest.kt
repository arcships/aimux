package ai.arcships.aimux.internal

import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test

class FlowBridgeTest {
    @Test
    fun `normal completion preserves ordering and runs on IO`() = runBlocking {
        val collector = Thread.currentThread()
        val aborts = AtomicInteger()
        val stream = FlowBridge.flow<Int>(capacity = 2, abort = { aborts.incrementAndGet() }) { send ->
            assertNotSame(collector, Thread.currentThread())
            repeat(100) { send(it) }
        }
        withTimeout(5000) {
            assertEquals((0 until 100).toList(), stream.toList())
        }
        assertEquals(0, aborts.get())
    }

    @Test
    fun `take one aborts and waits for native return`() = runBlocking {
        val aborts = AtomicInteger()
        val release = CountDownLatch(1)
        val returned = CountDownLatch(1)
        val stream = FlowBridge.flow<Int>(capacity = 1, abort = {
            aborts.incrementAndGet()
            release.countDown()
        }) { send ->
            try {
                send(1)
                assertTrue(release.await(5, TimeUnit.SECONDS))
                send(2) // Late callbacks after cancellation cannot escape the bridge.
            } finally {
                returned.countDown()
            }
        }
        withTimeout(5000) { assertEquals(listOf(1), stream.take(1).toList()) }
        assertEquals(1, aborts.get())
        assertEquals(0L, returned.count)
    }

    @Test
    fun `full queue blocks producer and cancellation releases it`() = runBlocking {
        val entered = CompletableDeferred<Unit>()
        val full = CompletableDeferred<Unit>()
        val returned = CountDownLatch(1)
        val sends = AtomicInteger()
        val aborts = AtomicInteger()
        val job = launch {
            FlowBridge.flow<Int>(capacity = 1, abort = { aborts.incrementAndGet() }) { send ->
                try {
                    send(1)
                    // Wait until the collector holds the first event.
                    runBlocking { entered.await() }
                    send(2)
                    sends.incrementAndGet()
                    full.complete(Unit)
                    send(3)
                    sends.incrementAndGet()
                } finally {
                    returned.countDown()
                }
            }.collect {
                entered.complete(Unit)
                awaitCancellation()
            }
        }
        withTimeout(5000) {
            full.await()
            assertEquals(1, sends.get())
            job.cancelAndJoin()
        }
        assertEquals(1, aborts.get())
        assertEquals(0L, returned.count)
    }

    @Test
    fun `producer exception is delivered once after buffered events`() = runBlocking {
        val failure = IllegalStateException("native failed")
        val errors = mutableListOf<Throwable>()
        val aborts = AtomicInteger()
        val values = withTimeout(5000) {
            FlowBridge.flow<Int>(capacity = 1, abort = {
                aborts.incrementAndGet()
                throw IllegalArgumentException("secondary abort failure")
            }) { send ->
                send(7)
                throw failure
            }.catch { errors.add(it) }.toList()
        }
        assertEquals(listOf(7), values)
        assertEquals(listOf(failure), errors)
        assertEquals(1, aborts.get())
    }

    @Test
    fun `collector failure is preserved and aborts once`() = runBlocking {
        val failure = IllegalArgumentException("consumer failed")
        val aborts = AtomicInteger()
        var caught: Throwable? = null
        try {
            withTimeout(5000) {
                FlowBridge.flow<Int>(capacity = 1, abort = { aborts.incrementAndGet() }) { send ->
                    repeat(10) { send(it) }
                    throw IllegalStateException("later native failure")
                }.collect { throw failure }
            }
        } catch (error: Throwable) {
            caught = error
        }
        assertSame(failure, caught)
        assertEquals(1, aborts.get())
    }

    @Test
    fun `unbounded and invalid capacities are rejected`() {
        for (capacity in listOf(-1, 0, 257, Int.MAX_VALUE)) {
            assertThrows(IllegalArgumentException::class.java) {
                FlowBridge.flow<Int>(capacity, abort = {}) {}
            }
        }
    }
}
