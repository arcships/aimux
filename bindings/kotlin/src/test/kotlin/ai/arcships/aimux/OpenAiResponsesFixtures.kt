package ai.arcships.aimux

import org.json.JSONArray
import org.json.JSONObject

/** Wire fixtures for the Responses model selected by the OpenAI FFI factory. */
internal object OpenAiResponsesFixtures {
    private fun usage(inputTokens: Int, outputTokens: Int) = JSONObject()
        .put("input_tokens", inputTokens)
        .put("output_tokens", outputTokens)
        .put("total_tokens", inputTokens + outputTokens)
        .put("input_tokens_details", JSONObject().put("cached_tokens", 0))
        .put("output_tokens_details", JSONObject().put("reasoning_tokens", 0))

    private fun response(output: JSONArray, inputTokens: Int, outputTokens: Int) = JSONObject()
        .put("id", "resp-test")
        .put("object", "response")
        .put("created_at", 1)
        .put("model", "gpt-4o")
        .put("status", "completed")
        .put("output", output)
        .put("usage", usage(inputTokens, outputTokens))

    private fun message(text: String) = JSONObject()
        .put("id", "msg-test")
        .put("type", "message")
        .put("role", "assistant")
        .put("status", "completed")
        .put("content", JSONArray().put(JSONObject()
            .put("type", "output_text")
            .put("text", text)
            .put("annotations", JSONArray())))

    private fun toolCall(callId: String, arguments: String) = JSONObject()
        .put("id", "fc-test")
        .put("type", "function_call")
        .put("call_id", callId)
        .put("name", "get_weather")
        .put("arguments", arguments)
        .put("status", "completed")

    fun text(text: String, inputTokens: Int = 10, outputTokens: Int = 8): String =
        response(JSONArray().put(message(text)), inputTokens, outputTokens).toString()

    fun toolCall(): String = response(
        JSONArray().put(toolCall("call_abc", "{\"location\":\"Tokyo\"}")), 20, 10,
    ).toString()

    private fun event(type: String, payload: JSONObject): String =
        "event: $type\ndata: ${payload.put("type", type)}\n\n"

    fun toolCallStream(): String {
        val item = toolCall("call_xyz", "{\"location\":\"Tokyo\"}")
        return event("response.created", JSONObject().put("response",
                response(JSONArray(), 0, 0).put("status", "in_progress"))) +
            event("response.output_item.added", JSONObject().put("output_index", 0)
                .put("item", toolCall("call_xyz", "").put("status", "in_progress"))) +
            event("response.function_call_arguments.delta", JSONObject()
                .put("item_id", "fc-test").put("output_index", 0).put("delta", "{\"location\":")) +
            event("response.function_call_arguments.delta", JSONObject()
                .put("item_id", "fc-test").put("output_index", 0).put("delta", "\"Tokyo\"}")) +
            event("response.output_item.done", JSONObject().put("output_index", 0).put("item", item)) +
            event("response.completed", JSONObject().put("response", response(JSONArray().put(item), 5, 2)))
    }
}
