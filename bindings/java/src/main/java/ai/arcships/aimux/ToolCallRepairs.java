package ai.arcships.aimux;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import com.sun.jna.Pointer;
import com.sun.jna.ptr.PointerByReference;

import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.Callable;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.FutureTask;

/**
 * Host-side tool-call repair (RFC-0035): the three pure native functions
 * and the post-processing loops that drive them.
 *
 * <p>Repair is pure post-processing — generation runs with no hook at all, the
 * invalid calls come back as data, and these functions re-validate and patch
 * the JSON before {@link TypedModel} decodes it.
 */
final class ToolCallRepairs {

    private static final String UNCHANGED = "{\"type\":\"unchanged\"}";

    private ToolCallRepairs() {}

    // ── Native wrappers ──────────────────────────────────────────────────────

    /** The AI SDK repair argument for one invalid call, or {@code "null"} when it has no tool set. */
    static String context(String toolCallJson, String promptJson, String optsJson) {
        PointerByReference out = new PointerByReference();
        Pointer e = AimuxFFI.INSTANCE.aimux_tool_call_repair_context(
            toolCallJson, promptJson, optsJson, out);
        return AimuxResult.extractString(e, out, "tool_call_repair_context");
    }

    /** Resolve one invalid call against a reply; returns the resulting ToolCall JSON. */
    static String apply(String toolCallJson, String optsJson, String replyJson) {
        PointerByReference out = new PointerByReference();
        Pointer e = AimuxFFI.INSTANCE.aimux_apply_tool_call_repair(
            toolCallJson, optsJson, replyJson, out);
        return AimuxResult.extractString(e, out, "apply_tool_call_repair");
    }

    /** Patch a whole result document (tool_calls and response_messages). */
    static String applyToResult(String resultJson, String optsJson, String toolCallId, String replyJson) {
        PointerByReference out = new PointerByReference();
        Pointer e = AimuxFFI.INSTANCE.aimux_apply_tool_call_repair_to_result(
            resultJson, optsJson, toolCallId, replyJson, out);
        return AimuxResult.extractString(e, out, "apply_tool_call_repair_to_result");
    }

    // ── Host loops ───────────────────────────────────────────────────────────

    /**
     * Repair every invalid tool call in a serialized GenerateTextResult /
     * GenerateObjectResult / StreamTextResultAggregated, at most once each.
     *
     * @return The patched result JSON, or {@code resultJson} unchanged when
     *         there is no hook or nothing to repair.
     */
    static String repairResult(String resultJson, String promptJson, String optsJson, ToolCallRepair hook) {
        if (hook == null) {
            return resultJson;
        }
        JsonNode calls = toolCalls(parse(resultJson));
        // Snapshot the invalid calls first: a repair may rename a call, and
        // patching rebuilds the document under us.
        List<JsonNode> invalid = new ArrayList<>();
        for (JsonNode call : calls) {
            if (call.path("invalid").asBoolean(false)) {
                invalid.add(call);
            }
        }
        String patched = resultJson;
        for (JsonNode call : invalid) {
            String callJson = call.toString();
            String contextJson = context(callJson, promptJson, optsJson);
            if (isNull(contextJson)) {
                continue; // no tool set — never repaired (AI SDK rule)
            }
            patched = applyToResult(patched, optsJson,
                call.path("tool_call_id").asText(), reply(hook, contextJson));
        }
        return patched;
    }

    /**
     * Repair an invalid tool-call stream part. Every other part — tool input
     * deltas included — passes straight through, as in the AI SDK.
     *
     * @param model The model being streamed, whose read hold is lent to the
     *              repair worker thread (may be {@code null} in unit tests that
     *              drive this loop without a model).
     * @return The part JSON to deliver.
     */
    static String repairStreamPart(String partJson, String promptJson, String optsJson,
                                   ToolCallRepair hook, Model model) {
        if (hook == null) {
            return partJson;
        }
        JsonNode part = parse(partJson);
        JsonNode call = part.path("ToolCall");
        if (!call.isObject() || !call.path("invalid").asBoolean(false)) {
            return partJson;
        }
        String contextJson = context(call.toString(), promptJson, optsJson);
        if (isNull(contextJson)) {
            return partJson;
        }
        String repaired = apply(call.toString(), optsJson, reply(offCallbackThread(hook, model), contextJson));
        ObjectNode out = Types.AimuxJson.MAPPER.createObjectNode();
        out.set("ToolCall", parse(repaired));
        return out.toString();
    }

    // ── Helpers ──────────────────────────────────────────────────────────────

    /**
     * Wrap a hook so it runs on its own thread.
     *
     * <p>Invariant: the FFI re-entrancy guard is THREAD-LOCAL, and a stream
     * part callback runs on the thread that entered {@code aimux_stream_text},
     * still inside that call. A hook doing the canonical thing — asking a model
     * to fix the arguments — would therefore fail with
     * {@code AIMUX_E_FFI_REENTRANT_CALL}. Only the three repair functions
     * themselves are safe on that thread, because they are pure.
     *
     * <p>The callback thread blocks on the result, so stream part ordering is
     * unchanged. Only the stream path needs this: by the time the non-streaming
     * loop runs, the native call has already returned.
     *
     * <p>Because that thread stays blocked holding {@code model}'s read lock,
     * the worker runs with the hold lent to it ({@link Model#withLentReadHold})
     * so a hook calling back into the SAME model does not queue behind a
     * pending {@code close()} — which would deadlock the three threads against
     * each other. The worker dies with the repair, so the lend cannot leak.
     */
    private static ToolCallRepair offCallbackThread(final ToolCallRepair hook, final Model model) {
        return context -> {
            final Callable<Types.RawToolCall> repair = () -> hook.repair(context);
            Callable<Types.RawToolCall> work = repair;
            if (model != null) {
                work = () -> model.withLentReadHold(repair);
            }
            FutureTask<Types.RawToolCall> task = new FutureTask<>(work);
            Thread thread = new Thread(task, "aimux-tool-call-repair");
            thread.setDaemon(true);
            thread.start();
            try {
                return task.get();
            } catch (ExecutionException e) {
                // Rethrow what the hook threw, so "throwing means failed" still
                // reports the host's own message.
                Throwable cause = e.getCause();
                if (cause instanceof Error) {
                    throw (Error) cause;
                }
                throw (Exception) cause;
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                throw e;
            }
        };
    }

    /** Run the host's hook over one repair context and encode its outcome as a reply. */
    private static String reply(ToolCallRepair hook, String contextJson) {
        Types.ToolCallRepairContext context;
        try {
            context = Types.AimuxJson.MAPPER.readValue(contextJson, Types.ToolCallRepairContext.class);
        } catch (IOException e) {
            throw new IllegalStateException(
                "aimux: failed to decode ToolCallRepairContext: " + e.getMessage(), e);
        }
        Types.RawToolCall replacement;
        try {
            replacement = hook.repair(context);
        } catch (Exception e) {
            // The host's own failure is data, not a thrown error: it comes back
            // as ToolCallRepair { original_error, cause }.
            ObjectNode failed = Types.AimuxJson.MAPPER.createObjectNode();
            failed.put("type", "failed");
            failed.put("message", e.getMessage() == null ? e.toString() : e.getMessage());
            return failed.toString();
        }
        if (replacement == null) {
            return UNCHANGED;
        }
        ObjectNode repaired = Types.AimuxJson.MAPPER.createObjectNode();
        repaired.put("type", "repaired");
        repaired.set("tool_call", Types.AimuxJson.MAPPER.valueToTree(replacement));
        return repaired.toString();
    }

    /** {@code tool_calls}, from a text result or from the {@code raw} of an object result. */
    private static JsonNode toolCalls(JsonNode result) {
        JsonNode calls = result.path("tool_calls");
        return calls.isArray() ? calls : result.path("raw").path("tool_calls");
    }

    private static boolean isNull(String json) {
        return parse(json).isNull();
    }

    private static JsonNode parse(String json) {
        try {
            return Types.AimuxJson.MAPPER.readTree(json);
        } catch (IOException e) {
            throw new IllegalStateException("aimux: failed to parse JSON: " + e.getMessage(), e);
        }
    }
}
