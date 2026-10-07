package ai.arcships.aimux

import java.io.Closeable
import java.util.concurrent.atomic.AtomicLong

/** Official TypeSafe Jev decisions using the core JSON request/result contract. */
class DecisionModel private constructor(handle: Long) : Closeable {
    private val handle = AtomicLong(handle)

    companion object {
        /** Endpoint is a complete POST URL; null uses the official API. */
        @JvmStatic
        @JvmOverloads
        fun jev(apiKey: String, modelId: String, endpoint: String? = null,
                probabilitySource: String? = null): DecisionModel =
            DecisionModel(handleResult("jev decision") { out ->
                FFI.lib.aimux_jev_decision_new_with_probability_source(apiKey, modelId, endpoint, probabilitySource, out)
            })
    }

    /** Optional abort handle follows the existing C ABI signal lifecycle. */
    @JvmOverloads
    fun decide(optsJson: String, abortHandle: Long = 0L): String {
        requireJsonRequired("optsJson", optsJson)
        return stringResult("decide") { out ->
            FFI.lib.aimux_decide_with_abort(requireHandle(), optsJson, abortHandle, out)
        }
    }

    /** Query capabilities and rounding precision without HTTP. */
    fun capabilities(): String = stringResult("decision capabilities") { out ->
        FFI.lib.aimux_decision_capabilities(requireHandle(), out)
    }

    private fun requireHandle(): Long = handle.get().also {
        if (it == 0L) throw IllegalStateException("DecisionModel is closed")
    }

    override fun close() {
        val value = handle.getAndSet(0L)
        if (value != 0L) FFI.lib.aimux_drop_handle(value)
    }

    protected fun finalize() { close() }
}
