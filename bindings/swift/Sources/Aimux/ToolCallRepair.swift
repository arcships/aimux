// ToolCallRepair.swift — host-side tool-call repair (RFC-0035).
//
// aimux-core is single-call: an unparseable tool call never fails generation,
// it comes back as a `ToolCall` with `invalid: true` and `error`. Repair is
// therefore post-processing done here, after the generation call returns: the
// binding asks the library for the AI SDK `repairToolCall` argument, hands it
// to the user's closure, and feeds the answer back into a pure native function
// that re-validates and patches. Only JSON crosses the boundary — nothing is
// registered with the C ABI and no native call is in flight while the closure
// runs, so the closure may itself call back into aimux.
//
// OpenAI-shaped output has no `invalid` / `error` to repair from, so
// `generateTextAsOpenAI` repairs the native result and converts it
// (`aimux_generate_text_result_as_openai`); `streamTextAsOpenAI` does not
// reflect repair (RFC-0035 §4).

import CAimuxFFI
import Foundation

// ─────────────────────────────────────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────────────────────────────────────

/// A tool call as the provider produced it (`aimux_core::RawToolCall`).
///
/// Field-for-field the same as ``ToolCall``, except `input` is the model's raw
/// argument *text* rather than a parsed value — that text is exactly what a
/// repair function needs to see, and what it returns a corrected version of.
public struct RawToolCall: Codable, Equatable {
    public var toolCallId: String
    public var toolName: String
    /// The model's raw argument text, e.g. `#"{"city":"Singapore"}"#`.
    public var input: String
    public var providerExecuted: Bool?
    public var dynamic: Bool?
    public var thoughtSignature: String?
    public var providerMetadata: JSONValue?

    enum CodingKeys: String, CodingKey {
        case toolCallId = "tool_call_id"
        case toolName = "tool_name"
        case input
        case providerExecuted = "provider_executed"
        case dynamic
        case thoughtSignature = "thought_signature"
        case providerMetadata = "provider_metadata"
    }

    public init(toolCallId: String, toolName: String, input: String,
                providerExecuted: Bool? = nil, dynamic: Bool? = nil,
                thoughtSignature: String? = nil, providerMetadata: JSONValue? = nil) {
        self.toolCallId = toolCallId; self.toolName = toolName; self.input = input
        self.providerExecuted = providerExecuted; self.dynamic = dynamic
        self.thoughtSignature = thoughtSignature; self.providerMetadata = providerMetadata
    }
}

/// The argument handed to ``GenerateTextOptions/repairToolCall``. Mirrors the
/// AI SDK `repairToolCall` argument.
public struct ToolCallRepairContext: Codable, Equatable {
    /// The failing call, with the provider's raw argument text.
    public var toolCall: RawToolCall
    /// The failure the call already carries, as externally-tagged wire JSON
    /// (the same encoding as ``ToolCall/error``), e.g.
    /// `{"InvalidToolInput":{"tool_name":…,"tool_input":…,"cause":…}}`.
    public var error: JSONValue
    /// JSON Schema of the named tool. A name that does not resolve to a
    /// function tool yields the AI SDK's default empty-object schema.
    public var inputSchema: JSONValue
    /// The tool set the call was generated with.
    public var tools: [Tool]
    /// The messages the model saw, derived from the same prompt.
    public var messages: [ModelMessage]
    /// The instructions the call was generated with, if any.
    public var instructions: String?

    enum CodingKeys: String, CodingKey {
        case toolCall = "tool_call"
        case error
        case inputSchema = "input_schema"
        case tools, messages, instructions
    }

    public init(toolCall: RawToolCall, error: JSONValue, inputSchema: JSONValue,
                tools: [Tool], messages: [ModelMessage], instructions: String? = nil) {
        self.toolCall = toolCall; self.error = error; self.inputSchema = inputSchema
        self.tools = tools; self.messages = messages; self.instructions = instructions
    }
}

/// A one-shot repair function: return a corrected ``RawToolCall``, `nil` to
/// leave the call invalid as it is, or throw to fail the repair.
public typealias RepairToolCall = (ToolCallRepairContext) throws -> RawToolCall?

/// Carries the repair closure inside ``GenerateTextOptions`` without costing it
/// its synthesized `Codable` / `Equatable`. Two option values never differ by
/// their repair function: a closure has no equality worth comparing, and the
/// function is host-side only — it is never serialized into the options JSON.
struct RepairToolCallBox: Equatable {
    let run: RepairToolCall
    static func == (_: RepairToolCallBox, _: RepairToolCallBox) -> Bool { true }
}

// ─────────────────────────────────────────────────────────────────────────────
// The three native functions (pure, no handle, no I/O)
// ─────────────────────────────────────────────────────────────────────────────

/// Build the repair argument for one invalid tool call.
///
/// - Parameters:
///   - toolCall: One `GenerateTextResult.tool_calls` entry with `"invalid": true`.
///   - prompt: The prompt JSON the call was generated with.
///   - options: The options JSON the call was generated with.
/// - Returns: The serialized ``ToolCallRepairContext``, or the JSON literal
///   `"null"` when the options carried no tool set — such a call is never
///   repaired (AI SDK rule), and that is a success, not a failure.
public func toolCallRepairContext(
    toolCall: String, prompt: String, options: String? = nil
) throws -> String {
    try validateJson(toolCall, parameter: "toolCall")
    try validateJson(prompt, parameter: "prompt")
    try validateJson(options, parameter: "options")
    return try ffiStringCall { aimux_tool_call_repair_context(toolCall, prompt, options, $0) }
}

/// Resolve one invalid tool call against a repair reply, returning the
/// serialized ``ToolCall`` — valid, or invalid carrying a nested
/// `ToolCallRepair` error.
///
/// Throws `AimuxError.invalidArgument` when the options carry no tool set (for
/// such a call ``toolCallRepairContext(toolCall:prompt:options:)`` already
/// answered `null`) or when `toolCall` is not an invalid call.
public func applyToolCallRepair(
    toolCall: String, options: String?, reply: String
) throws -> String {
    try validateJson(toolCall, parameter: "toolCall")
    try validateJson(options, parameter: "options")
    try validateJson(reply, parameter: "reply")
    return try ffiStringCall { aimux_apply_tool_call_repair(toolCall, options, reply, $0) }
}

/// Apply a repair reply to a serialized `GenerateTextResult` or
/// `GenerateObjectResult`. Both `tool_calls` and the matching
/// `response_messages` tool-call part are rewritten — the transcript must keep
/// pointing at the same entry, or the next turn replays the unrepaired
/// arguments.
public func applyToolCallRepairToResult(
    result: String, options: String?, toolCallId: String, reply: String
) throws -> String {
    try validateJson(result, parameter: "result")
    try validateJson(options, parameter: "options")
    try validateJson(reply, parameter: "reply")
    return try ffiStringCall {
        aimux_apply_tool_call_repair_to_result(result, options, toolCallId, reply, $0)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The host loop — one helper for results, one for stream parts
// ─────────────────────────────────────────────────────────────────────────────

/// Repair every invalid tool call in a serialized result document and return
/// the patched JSON (the input unchanged when no repair function is set).
///
/// Handles `GenerateTextResult` and `StreamTextResultAggregated` (top-level
/// `tool_calls`) as well as `GenerateObjectResult` (`raw.tool_calls`). Calls
/// are visited in document order and each gets at most one repair attempt, as
/// in the AI SDK.
func repairedResultJson(
    _ resultJson: String, promptJson: String, optsJson: String?,
    options: GenerateTextOptions?
) throws -> String {
    guard let repair = options?.repairToolCall else { return resultJson }
    guard let document = (try? JSONSerialization.jsonObject(with: Data(resultJson.utf8)))
        as? [String: Any] else { return resultJson }
    // A GenerateObjectResult keeps the whole text result under `raw`.
    let target = document["tool_calls"] != nil ? document : document["raw"] as? [String: Any]
    guard let calls = target?["tool_calls"] as? [[String: Any]] else { return resultJson }

    var patched = resultJson
    for call in calls where call["invalid"] as? Bool == true {
        guard let id = call["tool_call_id"] as? String, let callJson = jsonString(call) else { continue }
        let contextJson = try toolCallRepairContext(
            toolCall: callJson, prompt: promptJson, options: optsJson
        )
        guard let context = try decodeRepairContext(contextJson) else { continue }
        patched = try applyToolCallRepairToResult(
            result: patched, options: optsJson, toolCallId: id,
            reply: repairReply(context, repair)
        )
    }
    return patched
}

/// Repair a `{"ToolCall": …}` stream part, returning the replacement part JSON.
///
/// Any other part — `ToolInputDelta` above all — is returned untouched and
/// immediately: the AI SDK forwards argument deltas verbatim even when a repair
/// function is configured.
func repairedStreamPartJson(
    _ partJson: String, promptJson: String, optsJson: String?,
    options: GenerateTextOptions?
) throws -> String {
    guard let repair = options?.repairToolCall,
          let part = (try? JSONSerialization.jsonObject(with: Data(partJson.utf8))) as? [String: Any],
          let call = part["ToolCall"] as? [String: Any],
          call["invalid"] as? Bool == true,
          let callJson = jsonString(call)
    else { return partJson }

    // The two library calls are pure — no runtime, no `ffi_block_on` — so they
    // are safe right here on the callback thread. Only the user's closure has
    // to be moved off it.
    let contextJson = try toolCallRepairContext(
        toolCall: callJson, prompt: promptJson, options: optsJson
    )
    guard let context = try decodeRepairContext(contextJson) else { return partJson }
    let reply = offCallbackThread { repairReply(context, repair) }
    let repairedJson = try applyToolCallRepair(
        toolCall: callJson, options: optsJson, reply: reply
    )
    let repaired = try JSONSerialization.jsonObject(with: Data(repairedJson.utf8))
    return jsonString(["ToolCall": repaired]) ?? partJson
}

/// Run `body` on another thread and block until it answers.
///
/// A dedicated `Thread`, not `DispatchQueue.sync`: GCD runs a `sync` block on
/// the calling thread when it can, which would put the hook right back on the
/// guarded callback thread.
///
/// Invariant: the FFI re-entrancy guard is **thread-local**. A stream
/// `on_part` callback runs on the very thread that entered
/// `aimux_stream_text`, still inside that call's `ffi_block_on`, so any aimux
/// entry point invoked on it comes straight back as
/// `AIMUX_E_FFI_REENTRANT_CALL` (204). A repair function is expected to ask a
/// model to fix the arguments, so it must not run there.
///
/// Ordering is unaffected: the callback thread waits here, so the repaired
/// part is still delivered in its place in the stream, before the next one.
private func offCallbackThread<T>(_ body: @escaping () -> T) -> T {
    var answer: T?
    let done = DispatchSemaphore(value: 0)
    let worker = Thread {
        answer = body()
        done.signal()
    }
    worker.start()
    done.wait()
    // `body` is non-throwing and always assigns before signalling.
    return answer!
}

/// Decode a repair context, or `nil` for the JSON literal `null` the library
/// writes for a call made without a tool set (skip it — never repairable).
private func decodeRepairContext(_ json: String) throws -> ToolCallRepairContext? {
    guard json.trimmingCharacters(in: .whitespacesAndNewlines) != "null" else { return nil }
    return try JSONDecoder().decode(ToolCallRepairContext.self, from: Data(json.utf8))
}

/// Run the user's repair function and phrase its outcome as a reply:
/// a replacement call, `nil` (leave the call as it is), or a throw (the call
/// stays invalid, carrying a `ToolCallRepair` error with this message as cause).
private func repairReply(_ context: ToolCallRepairContext, _ repair: RepairToolCall) -> String {
    do {
        guard let replacement = try repair(context) else { return #"{"type":"unchanged"}"# }
        let encoded = try JSONEncoder().encode(replacement)
        return #"{"type":"repaired","tool_call":"# + String(decoding: encoded, as: UTF8.self) + "}"
    } catch {
        let message = (error as? LocalizedError)?.errorDescription ?? String(describing: error)
        return jsonString(["type": "failed", "message": message])
            ?? #"{"type":"failed","message":"repair function failed"}"#
    }
}

/// Serialize a JSONSerialization-compatible value back to a JSON string.
private func jsonString(_ object: Any) -> String? {
    guard let data = try? JSONSerialization.data(withJSONObject: object, options: [.fragmentsAllowed])
    else { return nil }
    return String(data: data, encoding: .utf8)
}
