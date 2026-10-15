package ai.arcships.aimux.kotlin

import ai.arcships.aimux.NativeException
import ai.arcships.aimux.internal.FlowBridge
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.asExecutor
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.emitAll
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import java.util.concurrent.CompletionException
import java.util.concurrent.ExecutionException
import java.util.concurrent.Executor
import java.util.concurrent.Future
import java.util.concurrent.Phaser
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import java.util.function.Consumer
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/** Start and await a Java call on IO. Cancellation signals Java's owned Abort. */
suspend fun <T> aimuxCall(start: (Executor) -> Future<T>): T = withContext(Dispatchers.IO) {
    var future: Future<T>? = null
    try {
        currentCoroutineContext().ensureActive()
        future = start(Dispatchers.IO.asExecutor())
        currentCoroutineContext().ensureActive()
        future.awaitAimux()
    } catch (error: Throwable) {
        throw projectError(error)
    } finally {
        if (!currentCoroutineContext().isActive && future?.isCancelled != true) future?.cancel(true)
    }
}

/** Await a Java operation; cancelling the coroutine cancels its Future. */
suspend fun <T> Future<T>.awaitAimux(): T = suspendCancellableCoroutine { continuation ->
    continuation.invokeOnCancellation { cancel(true) }
    Dispatchers.IO.asExecutor().execute {
        try {
            continuation.resume(get())
        } catch (error: Throwable) {
            continuation.resumeWithException(projectError(error))
        }
    }
}

/**
 * A cold, bounded stream over Java's Executor/Consumer API. [start] must schedule
 * all callback work on the supplied executor and return its Future promptly.
 * Collection waits for that work to return, including after cancellation, so
 * Java retains callback context through the native return boundary.
 */
fun <T> aimuxFlow(
    capacity: Int = 64,
    start: (Executor, Consumer<T>) -> Future<*>,
): Flow<T> {
    require(capacity in 1..256) { "capacity must be between 1 and 256" }
    return flow {
        val future = AtomicReference<Future<*>?>()
        val cancelled = AtomicBoolean()
        val tasks = Phaser(1)
        val executor = Executor { task ->
            tasks.register()
            try {
                Dispatchers.IO.asExecutor().execute {
                    try { task.run() } finally { tasks.arriveAndDeregister() }
                }
            } catch (error: Throwable) {
                tasks.arriveAndDeregister()
                throw error
            }
        }
        emitAll(FlowBridge.flow<T>(capacity, abort = {
            cancelled.set(true)
            future.get()?.cancel(true)
        }) { send ->
            try {
                val operation = start(executor, Consumer(send))
                future.set(operation)
                if (cancelled.get()) operation.cancel(true)
                operation.get()
            } finally {
                // A cancelled Future can finish before its native task returns.
                tasks.arriveAndAwaitAdvance()
            }
        })
    }.catch { throw projectError(it) }
}

internal fun projectError(error: Throwable): Throwable = when (error) {
    is ExecutionException, is CompletionException -> error.cause?.let(::projectError) ?: error
    is NativeException -> AimuxException.fromJava(error)
    else -> error
}
