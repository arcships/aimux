package ai.arcships.aimux;

import org.json.JSONArray;
import org.json.JSONObject;

/** Wire fixtures for the Responses model selected by the OpenAI FFI factory. */
final class OpenAiResponsesFixtures {
    private OpenAiResponsesFixtures() { }

    private static JSONObject usage(int inputTokens, int outputTokens) {
        return new JSONObject()
            .put("input_tokens", inputTokens)
            .put("output_tokens", outputTokens)
            .put("total_tokens", inputTokens + outputTokens)
            .put("input_tokens_details", new JSONObject().put("cached_tokens", 0))
            .put("output_tokens_details", new JSONObject().put("reasoning_tokens", 0));
    }

    private static JSONObject response(JSONArray output, int inputTokens, int outputTokens) {
        return new JSONObject()
            .put("id", "resp-test")
            .put("object", "response")
            .put("created_at", 1)
            .put("model", "gpt-4o")
            .put("status", "completed")
            .put("output", output)
            .put("usage", usage(inputTokens, outputTokens));
    }

    private static JSONObject message(String text) {
        return new JSONObject()
            .put("id", "msg-test")
            .put("type", "message")
            .put("role", "assistant")
            .put("status", "completed")
            .put("content", new JSONArray().put(new JSONObject()
                .put("type", "output_text")
                .put("text", text)
                .put("annotations", new JSONArray())));
    }

    private static JSONObject toolCall(String callId, String arguments) {
        return new JSONObject()
            .put("id", "fc-test")
            .put("type", "function_call")
            .put("call_id", callId)
            .put("name", "get_weather")
            .put("arguments", arguments)
            .put("status", "completed");
    }

    static String text(String text, int inputTokens, int outputTokens) {
        return response(new JSONArray().put(message(text)), inputTokens, outputTokens).toString();
    }

    static String toolCall() {
        return response(new JSONArray().put(toolCall("call_abc", "{\"location\":\"Tokyo\"}")),
            20, 10).toString();
    }

    private static String event(String type, JSONObject payload) {
        return "event: " + type + "\ndata: " + payload.put("type", type) + "\n\n";
    }

    static String textStream() {
        JSONObject item = message("Hello world");
        return event("response.created", new JSONObject().put("response",
                response(new JSONArray(), 0, 0).put("status", "in_progress")))
            + event("response.output_item.added", new JSONObject().put("output_index", 0)
                .put("item", message("").put("status", "in_progress")))
            + event("response.output_text.delta", new JSONObject().put("item_id", "msg-test")
                .put("output_index", 0).put("content_index", 0).put("delta", "Hello"))
            + event("response.output_text.delta", new JSONObject().put("item_id", "msg-test")
                .put("output_index", 0).put("content_index", 0).put("delta", " world"))
            + event("response.output_item.done", new JSONObject().put("output_index", 0).put("item", item))
            + event("response.completed", new JSONObject().put("response",
                response(new JSONArray().put(item), 5, 2)));
    }

    static String toolCallStream() {
        JSONObject item = toolCall("call_xyz", "{\"location\":\"Tokyo\"}");
        return event("response.created", new JSONObject().put("response",
                response(new JSONArray(), 0, 0).put("status", "in_progress")))
            + event("response.output_item.added", new JSONObject().put("output_index", 0)
                .put("item", toolCall("call_xyz", "").put("status", "in_progress")))
            + event("response.function_call_arguments.delta", new JSONObject()
                .put("item_id", "fc-test").put("output_index", 0).put("delta", "{\"location\":"))
            + event("response.function_call_arguments.delta", new JSONObject()
                .put("item_id", "fc-test").put("output_index", 0).put("delta", "\"Tokyo\"}"))
            + event("response.output_item.done", new JSONObject().put("output_index", 0).put("item", item))
            + event("response.completed", new JSONObject().put("response",
                response(new JSONArray().put(item), 5, 2)));
    }
}
