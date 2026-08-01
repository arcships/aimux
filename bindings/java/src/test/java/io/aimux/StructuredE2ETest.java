package io.aimux;

import org.json.JSONArray;
import org.json.JSONObject;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.TimeUnit;

import static org.assertj.core.api.Assertions.assertThat;
import static org.junit.jupiter.api.Assertions.assertTimeout;

/**
 * Structured end-to-end tests for the raw JSON {@link Model} layer (mirror of
 * the Kotlin binding's StructuredE2ETest.kt).
 *
 * <p>These spin up a local mock HTTP server speaking the OpenAI
 * chat-completions wire format, point an OpenAI {@code Model} at it via the
 * base-url constructor, and assert that the binding correctly:
 * <ol>
 *   <li>parses tool_calls out of a provider response,</li>
 *   <li>forwards multi-role messages,</li>
 *   <li>forwards {@code tool_choice},</li>
 *   <li>parses tool-call stream parts from an SSE stream,</li>
 *   <li>runs a full tool-call round-trip (tool_call + tool_result + final answer).</li>
 * </ol>
 *
 * No real network access is performed — every request hits 127.0.0.1.
 */
class StructuredE2ETest {

    private MockProviderServer server;

    @BeforeEach
    void setUp() {
        server = new MockProviderServer();
    }

    @AfterEach
    void tearDown() {
        server.close();
    }

    // ── canned OpenAI responses ───────────────────────────────────────────

    /** Plain OpenAI text response (no tool calls). */
    private static String plainOpenAiResponse() {
        return new JSONObject()
            .put("id", "chatcmpl-test")
            .put("model", "gpt-4o")
            .put("choices", new JSONArray().put(
                new JSONObject()
                    .put("message", new JSONObject()
                        .put("role", "assistant")
                        .put("content", "Rust is a systems programming language."))
                    .put("finish_reason", "stop")))
            .put("usage", new JSONObject()
                .put("prompt_tokens", 10)
                .put("completion_tokens", 8)
                .put("total_tokens", 18))
            .toString();
    }

    /** OpenAI response that requests a tool call (get_weather). */
    private static String toolCallOpenAiResponse() {
        return new JSONObject()
            .put("id", "chatcmpl-tc")
            .put("model", "gpt-4o")
            .put("choices", new JSONArray().put(
                new JSONObject()
                    .put("message", new JSONObject()
                        .put("role", "assistant")
                        .put("content", JSONObject.NULL)
                        .put("tool_calls", new JSONArray().put(
                            new JSONObject()
                                .put("id", "call_abc")
                                .put("type", "function")
                                .put("function", new JSONObject()
                                    .put("name", "get_weather")
                                    .put("arguments", "{\"location\":\"Tokyo\"}")))))
                    .put("finish_reason", "tool_calls")))
            .put("usage", new JSONObject()
                .put("prompt_tokens", 20)
                .put("completion_tokens", 10)
                .put("total_tokens", 30))
            .toString();
    }

    /** A plain text response used as the second leg of the tool-call round-trip. */
    private static String finalTextResponse() {
        return new JSONObject()
            .put("id", "chatcmpl-2")
            .put("model", "gpt-4o")
            .put("choices", new JSONArray().put(
                new JSONObject()
                    .put("message", new JSONObject()
                        .put("role", "assistant")
                        .put("content", "The weather in Tokyo is sunny."))
                    .put("finish_reason", "stop")))
            .put("usage", new JSONObject()
                .put("prompt_tokens", 30)
                .put("completion_tokens", 8)
                .put("total_tokens", 38))
            .toString();
    }

    /** OpenAI SSE stream: tool_calls name delta -> arguments delta -> finish -> [DONE]. */
    private static String toolCallSseResponse() {
        StringBuilder sb = new StringBuilder();
        sb.append("data: ").append(new JSONObject()
            .put("id", "1")
            .put("model", "gpt-4o")
            .put("choices", new JSONArray().put(
                new JSONObject().put("delta", new JSONObject()
                    .put("role", "assistant")
                    .put("tool_calls", new JSONArray().put(
                        new JSONObject()
                            .put("index", 0)
                            .put("id", "call_xyz")
                            .put("type", "function")
                            .put("function", new JSONObject()
                                .put("name", "get_weather")
                                .put("arguments", ""))))))))
            .append("\n\n");
        sb.append("data: ").append(new JSONObject()
            .put("id", "1")
            .put("model", "gpt-4o")
            .put("choices", new JSONArray().put(
                new JSONObject().put("delta", new JSONObject()
                    .put("tool_calls", new JSONArray().put(
                        new JSONObject()
                            .put("index", 0)
                            .put("function", new JSONObject()
                                .put("arguments", "{\"location\":\"Tokyo\"}"))))))))
            .append("\n\n");
        sb.append("data: ").append(new JSONObject()
            .put("id", "1")
            .put("model", "gpt-4o")
            .put("choices", new JSONArray().put(
                new JSONObject().put("delta", new JSONObject())
                    .put("finish_reason", "tool_calls")))
            .put("usage", new JSONObject()
                .put("prompt_tokens", 5)
                .put("completion_tokens", 2)
                .put("total_tokens", 7)))
            .append("\n\n");
        sb.append("data: [DONE]\n\n");
        return sb.toString();
    }

    /** A {@code tools} option describing a single get_weather function tool. */
    private static String weatherToolsOpts(String includeToolChoice) {
        JSONObject opts = new JSONObject();
        opts.put("tools", new JSONArray().put(
            new JSONObject()
                .put("type", "function")
                .put("name", "get_weather")
                .put("input_schema", new JSONObject()
                    .put("type", "object")
                    .put("properties", new JSONObject()
                        .put("location", new JSONObject().put("type", "string"))))));
        if (includeToolChoice != null) {
            opts.put("tool_choice", includeToolChoice);
        }
        return opts.toString();
    }

    private static String weatherToolsOpts() {
        return weatherToolsOpts(null);
    }

    // ── Tests ─────────────────────────────────────────────────────────────

    @Test
    void generateTextParsesToolCalls() {
        server.setResponseBody(toolCallOpenAiResponse());

        try (Model model = Model.openaiWithBase("sk-test-fake-key", "gpt-4o", server.baseUrl())) {
            String resultJson = model.generateText("\"What is the weather in Tokyo?\"", weatherToolsOpts());
            JSONObject result = new JSONObject(resultJson);

            // tool_calls[0].tool_name == "get_weather"
            JSONArray toolCalls = result.getJSONArray("tool_calls");
            assertThat(toolCalls.length()).isGreaterThan(0);
            JSONObject firstCall = toolCalls.getJSONObject(0);
            assertThat(firstCall.getString("tool_name")).isEqualTo("get_weather");
            assertThat(firstCall.getString("tool_call_id")).isEqualTo("call_abc");
            assertThat(firstCall.getJSONObject("input").getString("location")).isEqualTo("Tokyo");

            // raw.content contains a "ToolCall" variant.
            JSONArray rawContent = result.getJSONObject("raw").getJSONArray("content");
            boolean hasToolCallVariant = false;
            for (int i = 0; i < rawContent.length(); i++) {
                if (rawContent.getJSONObject(i).has("ToolCall")) {
                    hasToolCallVariant = true;
                    break;
                }
            }
            assertThat(hasToolCallVariant).isTrue();
        }
    }

    @Test
    void multiRoleMessagesReachProvider() {
        server.setResponseBody(plainOpenAiResponse());

        // [{role:"system",content:"You are a helpful assistant."},
        //  {role:"user",content:"What is Rust?"}]
        JSONArray prompt = new JSONArray()
            .put(new JSONObject().put("role", "system").put("content", "You are a helpful assistant."))
            .put(new JSONObject().put("role", "user").put("content", "What is Rust?"));

        try (Model model = Model.openaiWithBase("sk-test-fake-key", "gpt-4o", server.baseUrl())) {
            String resultJson = model.generateText(prompt.toString());
            JSONObject result = new JSONObject(resultJson);
            assertThat(result.getString("text"))
                .isEqualTo("Rust is a systems programming language.");

            // The provider received both messages (system + user).
            JSONObject reqBody = new JSONObject(server.lastRequestBody());
            assertThat(reqBody.getString("model")).isEqualTo("gpt-4o");
            JSONArray messages = reqBody.getJSONArray("messages");
            assertThat(messages.length()).isEqualTo(2);
            assertThat(messages.getJSONObject(0).getString("role")).isEqualTo("system");
            assertThat(messages.getJSONObject(1).getString("role")).isEqualTo("user");
        }
    }

    @Test
    void toolChoiceReachesProvider() {
        server.setResponseBody(toolCallOpenAiResponse());

        try (Model model = Model.openaiWithBase("sk-test-fake-key", "gpt-4o", server.baseUrl())) {
            model.generateText("\"What is the weather in Tokyo?\"", weatherToolsOpts("required"));

            JSONObject reqBody = new JSONObject(server.lastRequestBody());
            assertThat(reqBody.getString("tool_choice")).isEqualTo("required");
            // tools were forwarded too.
            assertThat(reqBody.has("tools")).isTrue();
        }
    }

    @Test
    void streamTextParsesToolCallStreamParts() {
        // SSE: tool_calls delta (name) -> arguments delta -> finish_reason.
        server.setContentType("text/event-stream");
        server.setResponseBody(toolCallSseResponse());

        try (Model model = Model.openaiWithBase("sk-test-fake-key", "gpt-4o", server.baseUrl())) {
            // Run the blocking FFI stream call on a separate thread so the JNA
            // callbacks (which attach to the calling thread) don't deadlock with
            // tokio's block_on. The main thread collects parts via a queue.
            LinkedBlockingQueue<String> parts = new LinkedBlockingQueue<>();
            Thread streamThread = new Thread(() ->
                model.streamText("\"What is the weather in Tokyo?\"", weatherToolsOpts(),
                    parts::add,
                    () -> { try { parts.put("__done__"); } catch (InterruptedException ignored) { } },
                    err -> { try { parts.put("__done__"); } catch (InterruptedException ignored) { } }));
            streamThread.setDaemon(true);
            streamThread.start();

            List<String> collected = new ArrayList<>();
            assertTimeout(Duration.ofSeconds(30), () -> {
                while (true) {
                    String part = parts.poll(30, TimeUnit.SECONDS);
                    if (part == null || "__done__".equals(part)) {
                        break;
                    }
                    collected.add(part);
                }
            });
            try {
                streamThread.join(5000);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                throw new RuntimeException(e);
            }

            assertThat(collected).isNotEmpty();
            // Stream must surface tool-call activity: either a complete ToolCall
            // part or ToolInputDelta deltas (or both).
            boolean hasToolPart = false;
            for (String part : collected) {
                if (part.contains("\"ToolCall\"") || part.contains("\"ToolInputDelta\"")) {
                    hasToolPart = true;
                    break;
                }
            }
            assertThat(hasToolPart)
                .as("expected a ToolCall or ToolInputDelta stream part; got: %s", collected)
                .isTrue();

            // The finish part should also be present, carrying the tool-calls
            // finish reason.
            boolean hasFinish = false;
            for (String part : collected) {
                if (part.contains("\"Finish\"")) {
                    hasFinish = true;
                    break;
                }
            }
            assertThat(hasFinish).isTrue();
        }
    }

    @Test
    void toolCallFullRoundTrip() {
        // A tool-call round-trip = two generateText calls with a ToolResult
        // back-filled between them. The mock server returns a tool-call
        // response for the first request, then a plain text response for the
        // second (FIFO queue).
        server.setResponses(toolCallOpenAiResponse(), finalTextResponse());

        try (Model model = Model.openaiWithBase("sk-test-fake-key", "gpt-4o", server.baseUrl())) {
            // 1st call: provider asks to call get_weather.
            String firstResultJson =
                model.generateText("\"What's the weather in Tokyo?\"", weatherToolsOpts());
            JSONObject firstResult = new JSONObject(firstResultJson);
            JSONArray toolCalls = firstResult.getJSONArray("tool_calls");
            assertThat(toolCalls.length()).isGreaterThan(0);
            JSONObject firstCall = toolCalls.getJSONObject(0);
            assertThat(firstCall.getString("tool_name")).isEqualTo("get_weather");
            assertThat(firstCall.getString("tool_call_id")).isEqualTo("call_abc");
            assertThat(firstCall.getJSONObject("input").getString("location"))
                .isEqualTo("Tokyo");

            // 2nd call: echo the full conversation back, including the
            // assistant's tool_call and the ToolResult we synthesised.
            // Input uses the engine's ContentPart variants (tool_call /
            // tool_result); the engine converts these to the OpenAI wire
            // format on the outbound request. NOTE: the tool_result part uses
            // the `result` field (never `output`) — the engine's
            // ContentPart::ToolResult requires it (aimux-core/content.rs).
            JSONArray messages = new JSONArray()
                .put(new JSONObject().put("role", "user").put("content", "What's the weather in Tokyo?"))
                .put(new JSONObject().put("role", "assistant")
                    .put("content", new JSONArray().put(
                        new JSONObject()
                            .put("type", "tool_call")
                            .put("tool_call_id", "call_abc")
                            .put("tool_name", "get_weather")
                            .put("input", new JSONObject().put("location", "Tokyo")))))
                .put(new JSONObject().put("role", "tool")
                    .put("content", new JSONArray().put(
                        new JSONObject()
                            .put("type", "tool_result")
                            .put("tool_call_id", "call_abc")
                            .put("result", new JSONObject()
                                .put("temperature", 22)
                                .put("condition", "sunny")))));

            String secondResultJson = model.generateText(messages.toString(), weatherToolsOpts());
            JSONObject secondResult = new JSONObject(secondResultJson);
            assertThat(secondResult.getString("text"))
                .isEqualTo("The weather in Tokyo is sunny.");

            // The provider received 3 messages; the last is the tool result,
            // carrying the matching tool_call_id.
            JSONObject reqBody = new JSONObject(server.lastRequestBody());
            JSONArray reqMessages = reqBody.getJSONArray("messages");
            assertThat(reqMessages.length()).isEqualTo(3);
            JSONObject lastMsg = reqMessages.getJSONObject(2);
            assertThat(lastMsg.getString("role")).isEqualTo("tool");
            assertThat(lastMsg.getString("tool_call_id")).isEqualTo("call_abc");
        }
    }
}

