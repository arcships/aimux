package ai.arcships.aimux.internal

import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking

/** Bridges a synchronous native call; its callbacks must finish before [produce] returns. */
internal object FlowBridge {
    /**
     * Each collector starts its own call on IO. A full channel blocks the native
     * callback without dropping values. [abort] must be bounded and unblock the
     * native call; collection retains the callback context until that call returns.
     * Payload byte limits remain the responsibility of the native operation.
     */
    fun <T> flow(
        capacity: Int = 64,
        abort: () -> Unit,
        produce: ((T) -> Unit) -> Unit,
    ): Flow<T> {
        require(capacity in 1..256) { "capacity must be between 1 and 256" }
        return flow {
            coroutineScope {
                val channel = Channel<T>(capacity)
                val failure = AtomicReference<Throwable?>()
                val aborted = AtomicBoolean()
                fun abortOnce() {
                    if (aborted.compareAndSet(false, true)) {
                        try {
                            abort()
                        } catch (error: Throwable) {
                            failure.compareAndSet(null, error)
                        }
                    }
                }
                launch(Dispatchers.IO) {
                    try {
                        produce { value ->
                            if (failure.get() == null) {
                                try {
                                    runBlocking { channel.send(value) }
                                } catch (error: Throwable) {
                                    // Never unwind through a native callback.
                                    failure.compareAndSet(null, error)
                                    abortOnce()
                                }
                            }
                        }
                    } catch (error: Throwable) {
                        failure.compareAndSet(null, error)
                        abortOnce()
                    } finally {
                        // Closing needs no queue slot and preserves buffered events.
                        channel.close(failure.get())
                    }
                }
                var drained = false
                try {
                    for (value in channel) emit(value)
                    drained = true
                } finally {
                    if (!drained) {
                        channel.cancel()
                        abortOnce()
                    }
                }
            }
        }
    }
}
