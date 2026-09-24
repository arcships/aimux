/**
 * aimux — host-side tool-call repair (RFC-0035).
 *
 * Core is single-call: an unparseable tool call never fails generation, it
 * comes back as `ToolCall(invalid = true, error = …)`. Repair therefore happens
 * here, after the call returns: [TypedModel] finds the invalid calls, runs the
 * caller's [RepairToolCall] on the JVM, and hands the answer back to three pure
 * native functions that re-validate and patch. Nothing but JSON crosses the C
 * ABI — no callbacks, no handles, no sessions — so a repair function is free to
 * call back into aimux (typically: ask a model to fix the arguments).
 */

package ai.arcships.aimux

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import java.util.concurrent.ExecutionException
import java.util.concurrent.FutureTask

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/**
 * A provider-shaped tool call, as the model emitted it.
 *
 * Mirrors core `RawToolCall`. Unlike [ToolCall], `input` is the **raw argument
 * text** (the string the provider sent), not a parsed object — that is what a
 * repair function reads and what it returns a replacement for.
 */
@Serializable
data class RawToolCall(
    @SerialName("tool_call_id") val toolCallId: String,
    @SerialName("tool_name") val toolName: String,
    val input: String,
    @SerialName("provider_executed") val providerExecuted: Boolean? = null,
    val dynamic: Boolean? = null,
    @SerialName("thought_signature") val thoughtSignature: String? = null,
    @SerialName("provider_metadata") val providerMetadata: JsonElement? = null,
)

/**
 * Everything a repair function is given about one invalid tool call.
 *
 * Mirrors the AI SDK `repairToolCall` argument. [error] is the serialized
 * `AiMuxError` that made the call invalid (e.g. `{"InvalidToolInput": …}`),
 * carried verbatim from the call rather than re-derived. [inputSchema] is the
 * JSON Schema of the named function tool, or the AI SDK's empty-object schema
 * when the name resolves to no function tool. [messages] and [instructions]
 * are the prompt the call was generated from.
 */
@Serializable
data class ToolCallRepairContext(
    @SerialName("tool_call") val toolCall: RawToolCall,
    val error: JsonElement,
    @SerialName("input_schema") val inputSchema: JsonElement,
    val tools: List<Tool> = emptyList(),
    val messages: List<ModelMessage> = emptyList(),
    val instructions: String? = null,
)

/**
 * A caller-supplied repair attempt for one invalid tool call.
 *
 * Return a replacement [RawToolCall] to have core re-parse and re-validate it,
 * or `null` to leave the call invalid with its original error. Throwing is
 * reported as a failed repair: the call stays invalid and carries a
 * [ToolCallRepairError] whose cause is the exception's message.
 *
 * Called on the thread that issued the generation, outside any native call, so
 * it may block and may itself call aimux.
 */
typealias RepairToolCall = (ToolCallRepairContext) -> RawToolCall?

// ─────────────────────────────────────────────────────────────────────────────
// Raw native entry points — JSON in, JSON out, no handle.
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Build the repair argument for one invalid tool call.
 *
 * @param toolCallJson One `tool_calls` entry with `"invalid": true`.
 * @param promptJson The prompt the call was generated with (the same string
 *   passed to `generate_text` / `stream_text`).
 * @param optsJson The options the call was generated with (same rule).
 * @return A JSON [ToolCallRepairContext], or the literal `null` when the call
 *   was made without a tool set — AI SDK never repairs those, so skip it.
 */
fun toolCallRepairContext(toolCallJson: String, promptJson: String, optsJson: String? = null): String {
    requireJsonRequired("toolCallJson", toolCallJson)
    requireJsonRequired("promptJson", promptJson)
    requireJson("optsJson", optsJson)
    return stringResult("toolCallRepairContext") { out ->
        FFI.lib.aimux_tool_call_repair_context(toolCallJson, promptJson, optsJson, out)
    }
}

/**
 * Resolve one invalid tool call against a repair reply, returning the final
 * `ToolCall` JSON — valid, or invalid carrying a nested `ToolCallRepair` error.
 *
 * @param replyJson `{"type":"repaired","tool_call":…}` | `{"type":"unchanged"}`
 *   | `{"type":"failed","message":…}`.
 * @throws InvalidArgumentError when the options carry no tools, or the call is
 *   not an invalid one.
 */
fun applyToolCallRepair(toolCallJson: String, optsJson: String?, replyJson: String): String {
    requireJsonRequired("toolCallJson", toolCallJson)
    requireJson("optsJson", optsJson)
    requireJsonRequired("replyJson", replyJson)
    return stringResult("applyToolCallRepair") { out ->
        FFI.lib.aimux_apply_tool_call_repair(toolCallJson, optsJson, replyJson, out)
    }
}

/**
 * Apply a repair reply to a whole serialized result (`GenerateTextResult`,
 * `GenerateObjectResult` via its nested `raw`, or `StreamTextResultAggregated`).
 *
 * Rewrites both `tool_calls` and the matching `response_messages` tool-call
 * part — the transcript must agree with the repaired call, or the next turn
 * sends the model the unrepaired arguments again.
 *
 * @throws InvalidArgumentError when `toolCallId` matches no entry, the matched
 *   call is valid, or the options carry no tools.
 */
fun applyToolCallRepairToResult(
    resultJson: String,
    optsJson: String?,
    toolCallId: String,
    replyJson: String,
): String {
    requireJsonRequired("resultJson", resultJson)
    requireJson("optsJson", optsJson)
    requireJsonRequired("replyJson", replyJson)
    return stringResult("applyToolCallRepairToResult") { out ->
        FFI.lib.aimux_apply_tool_call_repair_to_result(resultJson, optsJson, toolCallId, replyJson, out)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Host loop — one helper for results, one for stream parts.
// ─────────────────────────────────────────────────────────────────────────────

private const val REPLY_UNCHANGED = """{"type":"unchanged"}"""

/** Run the caller's function and encode its outcome as a `ToolCallRepairReply`. */
private fun repairReply(repair: RepairToolCall, contextJson: String): String {
    val context = try {
        AimuxJson.decodeFromString(ToolCallRepairContext.serializer(), contextJson)
    } catch (e: Exception) {
        throw InvalidArgumentError(
            "failed to decode ToolCallRepairContext: ${e.message ?: e::class.simpleName}",
            cause = e,
        )
    }
    return try {
        val replacement = repair(context) ?: return REPLY_UNCHANGED
        buildJsonObject {
            put("type", "repaired")
            put("tool_call", AimuxJson.encodeToJsonElement(RawToolCall.serializer(), replacement))
        }.toString()
    } catch (e: Exception) {
        // A host exception has no typed counterpart in core; only its message
        // survives, as the cause of ToolCallRepairError.
        buildJsonObject {
            put("type", "failed")
            put("message", e.message ?: e::class.simpleName ?: "tool call repair failed")
        }.toString()
    }
}

private fun isInvalid(call: JsonObject): Boolean =
    (call["invalid"] as? JsonPrimitive)?.booleanOrNull == true

/**
 * Give every invalid tool call in a result document one repair attempt, in
 * order, and return the patched result JSON.
 *
 * Ids are read from the document as it arrived: patching one call never
 * touches another's id, and one attempt per call is the AI SDK rule.
 */
internal fun repairToolCallsInResult(
    resultJson: String,
    promptJson: String,
    optsJson: String?,
    repair: RepairToolCall,
): String {
    val root = AimuxJson.parseToJsonElement(resultJson) as? JsonObject ?: return resultJson
    // GenerateObjectResult nests the text result under `raw`.
    val target = if (root.containsKey("tool_calls")) root else root["raw"] as? JsonObject ?: return resultJson
    val invalid = (target["tool_calls"] as? JsonArray)
        ?.mapNotNull { it as? JsonObject }
        ?.filter(::isInvalid)
        ?: return resultJson

    var patched = resultJson
    for (call in invalid) {
        val contextJson = toolCallRepairContext(call.toString(), promptJson, optsJson)
        if (AimuxJson.parseToJsonElement(contextJson) is JsonNull) continue
        val id = call["tool_call_id"]?.jsonPrimitive?.content ?: continue
        patched = applyToolCallRepairToResult(patched, optsJson, id, repairReply(repair, contextJson))
    }
    return patched
}

/**
 * Wrap [repair] so it runs on a fresh thread while the caller waits.
 *
 * Invariant: the FFI re-entrancy guard is a THREAD-LOCAL flag set on the
 * thread that entered `aimux_stream_text` — the very thread the stream
 * callbacks run on. Any aimux call that blocks on the runtime (every generate
 * / stream entry point) fails there with `AIMUX_E_FFI_REENTRANT_CALL` (204),
 * and asking a model to fix the arguments is the canonical repair. So the
 * hook must not run on the callback thread. The three repair functions
 * themselves are pure and stay where they are. Part ordering is unchanged:
 * the callback thread blocks until the hook returns.
 *
 * [model] is the streaming model, whose read hold is lent to the worker for
 * the duration of the hook: the callback thread is blocked in `task.get()`
 * still holding it, so the handle cannot be dropped, while a fresh read
 * acquisition here would queue behind a waiting `close()` writer (the lock is
 * fair) and deadlock all three. The lend covers [model] only, and only on the
 * worker thread — a thread the hook starts itself still takes the lock.
 */
internal fun offCallbackThread(model: Model, repair: RepairToolCall): RepairToolCall = { context ->
    val task = FutureTask { model.withLentReadHold { repair(context) } }
    // Daemon, like Java's: a hook that never returns must not keep the JVM up.
    Thread(task, "aimux-tool-call-repair").apply { isDaemon = true }.start()
    try {
        task.get()
    } catch (e: ExecutionException) {
        // Rethrow what the hook threw, so "throw means failed" still maps to
        // the hook's own message rather than to the wrapper's.
        throw e.cause ?: e
    } catch (e: InterruptedException) {
        // The repair becomes a failed reply; the caller's interrupt survives.
        Thread.currentThread().interrupt()
        throw e
    }
}

/**
 * Repair an invalid `ToolCall` stream part, returning the replacement part
 * JSON. Any other part (tool-input deltas included) is returned untouched, so
 * deltas still reach the caller immediately — AI SDK behaviour.
 */
internal fun repairToolCallStreamPart(
    partJson: String,
    promptJson: String,
    optsJson: String?,
    repair: RepairToolCall,
): String {
    // StreamPart is externally tagged: {"ToolCall": {…}}.
    val part = AimuxJson.parseToJsonElement(partJson) as? JsonObject ?: return partJson
    val call = part["ToolCall"] as? JsonObject ?: return partJson
    if (!isInvalid(call)) return partJson

    val contextJson = toolCallRepairContext(call.toString(), promptJson, optsJson)
    if (AimuxJson.parseToJsonElement(contextJson) is JsonNull) return partJson
    val repaired = applyToolCallRepair(call.toString(), optsJson, repairReply(repair, contextJson))
    return JsonObject(part + ("ToolCall" to AimuxJson.parseToJsonElement(repaired))).toString()
}
