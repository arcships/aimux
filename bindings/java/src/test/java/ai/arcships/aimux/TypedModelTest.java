package ai.arcships.aimux;

import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import org.json.JSONArray;
import org.json.JSONObject;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.time.Duration;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;
import java.util.stream.Collectors;

import static org.assertj.core.api.Assertions.assertThat;
import static org.assertj.core.api.Assertions.assertThatThrownBy;
import static org.junit.jupiter.api.Assertions.assertTimeout;

/**
 * Tests for the typed {@link TypedModel} wrapper over the raw JSON-string
 * {@link Model} (mirror of the Kotlin binding's TypedModelTest.kt).
 *
 * <p>Like {@link StructuredE2ETest}, these spin up the shared
 * {@link MockProviderServer} (OpenAI chat-completions wire format) and assert
 * that the typed wrapper:
 * <ol>
 *   <li>returns a {@link Types.GenerateTextResult} object (no manual JSON parsing),</li>
 *   <li>surfaces {@code .text}, {@code .toolCalls[0].toolName}, and {@code .raw.content},</li>
 *   <li>forwards typed {@code tools} / {@code tool_choice} onto the provider request,</li>
 *   <li>forwards multi-role {@link Types.ModelMessage} conversations,</li>
 *   <li>decodes SSE stream parts into typed {@link Types.StreamPart}s.</li>
 * </ol>
 *
 * No real network access — every request hits 127.0.0.1.
 */
class TypedModelTest {

    private MockProviderServer server;

    @BeforeEach
    void setUp() {
        server = new MockProviderServer();
    }

    @AfterEach
    void tearDown() {
        server.close();
    }

    // ── canned OpenAI responses (mirror StructuredE2ETest) ───────────────

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

    // ── typed builders ──────────────────────────────────────────────────

    /** A JSON Schema for the get_weather tool's `location` argument. */
    private static Types.Tool weatherTool() {
        ObjectMapper m = new ObjectMapper();
        ObjectNode schema = m.createObjectNode();
        schema.put("type", "object");
        ObjectNode props = m.createObjectNode();
        props.set("location", m.createObjectNode().put("type", "string"));
        schema.set("properties", props);
        return Types.Tool.Function.builder().name("get_weather").inputSchema(schema).build();
    }

    // ── Tests ───────────────────────────────────────────────────────────

    @Test
    void topLevelToolCallProviderMetadataRoundTrips() throws Exception {
        ObjectMapper mapper = new ObjectMapper();
        ObjectNode metadata = mapper.createObjectNode();
        metadata.set("google", mapper.createObjectNode().put("cache_id", "cache-1"));
        Types.ToolCall original = Types.ToolCall.builder()
            .toolCallId("call_1")
            .toolName("get_weather")
            .providerMetadata(metadata)
            .build();

        String json = Types.AimuxJson.MAPPER.writeValueAsString(original);
        Types.ToolCall decoded = Types.AimuxJson.MAPPER.readValue(json, Types.ToolCall.class);

        assertThat(json).contains("\"provider_metadata\"");
        assertThat(decoded.getProviderMetadata()).isEqualTo(metadata);
        assertThat(decoded).isEqualTo(original);
    }

    @Test
    void generateTextReturnsTypedGenerateTextResultWithTextAndRawContent() {
        server.setResponseBody(plainOpenAiResponse());

        try (TypedModel model =
                 TypedModel.openaiWithBase("sk-test-fake-key", "gpt-4o", server.baseUrl())) {
            Types.GenerateTextResult result = model.generateText("What is Rust?");

            // No manual parsing: the wrapper returned a typed object.
            assertThat(result).isInstanceOf(Types.GenerateTextResult.class);

            // .text is a plain String.
            assertThat(result.getText()).isEqualTo("Rust is a systems programming language.");

            // A plain-text response carries no tool calls.
            assertThat(result.getToolCalls()).isEmpty();

            // .raw.content is accessible as a list (no JSON digging required).
            assertThat(result.getRaw().getContent()).isNotEmpty();
        }
    }

    @Test
    void generateTextParsesToolCallsIntoTypedObjects() {
        server.setResponseBody(toolCallOpenAiResponse());

        Types.GenerateTextOptions options = Types.GenerateTextOptions.builder()
            .tools(Collections.singletonList(weatherTool()))
            .toolChoice(Types.ToolChoice.AUTO)
            .build();

        try (TypedModel model =
                 TypedModel.openaiWithBase("sk-test-fake-key", "gpt-4o", server.baseUrl())) {
            Types.GenerateTextResult result =
                model.generateText("What is the weather in Tokyo?", options);

            // .toolCalls[0].toolName / .toolCallId / .input — all typed.
            assertThat(result.getToolCalls()).hasSize(1);
            Types.ToolCall call = result.getToolCalls().get(0);
            assertThat(call.getToolName()).isEqualTo("get_weather");
            assertThat(call.getToolCallId()).isEqualTo("call_abc");
            // input is a JsonNode (tool arguments); inspect it directly.
            assertThat(call.getInput().get("location").asText()).isEqualTo("Tokyo");

            // .raw.content carries the ToolCall variant tag.
            assertThat(result.getRaw().hasContentVariant("ToolCall")).isTrue();
        }
    }

    @Test
    void typedToolsAndToolChoiceReachProvider() {
        server.setResponseBody(toolCallOpenAiResponse());

        Types.GenerateTextOptions options = Types.GenerateTextOptions.builder()
            .tools(Collections.singletonList(weatherTool()))
            .toolChoice(Types.ToolChoice.REQUIRED)
            .build();

        try (TypedModel model =
                 TypedModel.openaiWithBase("sk-test-fake-key", "gpt-4o", server.baseUrl())) {
            model.generateText("What is the weather in Tokyo?", options);

            // The serialized options crossed the JSON boundary as the engine's
            // snake_case shape with only the fields the caller set.
            JSONObject reqBody = new JSONObject(server.lastRequestBody());
            assertThat(reqBody.getString("tool_choice")).isEqualTo("required");
            assertThat(reqBody.has("tools")).isTrue();
            JSONArray tools = reqBody.getJSONArray("tools");
            assertThat(tools.length()).isEqualTo(1);
            JSONObject tool = tools.getJSONObject(0);
            // The engine forwards the typed tool to the provider in OpenAI's
            // wire format: {type:"function", function:{name, parameters}}.
            assertThat(tool.getString("type")).isEqualTo("function");
            assertThat(tool.getJSONObject("function").getString("name")).isEqualTo("get_weather");
            assertThat(tool.getJSONObject("function").getJSONObject("parameters").getString("type"))
                .isEqualTo("object");
        }
    }

    @Test
    void multiRoleModelMessageListReachesProvider() {
        server.setResponseBody(plainOpenAiResponse());

        List<Types.ModelMessage> messages = Arrays.asList(
            Types.ModelMessage.text(Types.Role.SYSTEM, "You are a helpful assistant."),
            Types.ModelMessage.text(Types.Role.USER, "What is Rust?"));

        try (TypedModel model =
                 TypedModel.openaiWithBase("sk-test-fake-key", "gpt-4o", server.baseUrl())) {
            Types.GenerateTextResult result = model.generateText(messages);

            assertThat(result.getText()).isEqualTo("Rust is a systems programming language.");

            // The provider received both messages (system + user), in order.
            JSONObject reqBody = new JSONObject(server.lastRequestBody());
            assertThat(reqBody.getString("model")).isEqualTo("gpt-4o");
            JSONArray reqMessages = reqBody.getJSONArray("messages");
            assertThat(reqMessages.length()).isEqualTo(2);
            assertThat(reqMessages.getJSONObject(0).getString("role")).isEqualTo("system");
            assertThat(reqMessages.getJSONObject(0).getString("content"))
                .isEqualTo("You are a helpful assistant.");
            assertThat(reqMessages.getJSONObject(1).getString("role")).isEqualTo("user");
            assertThat(reqMessages.getJSONObject(1).getString("content")).isEqualTo("What is Rust?");
        }
    }

    @Test
    void streamTextStreamDecodesTypedParts() {
        // SSE: tool_calls delta (name) -> arguments delta -> finish_reason.
        server.setContentType("text/event-stream");
        server.setResponseBody(toolCallSseResponse());

        Types.GenerateTextOptions options = Types.GenerateTextOptions.builder()
            .tools(Collections.singletonList(weatherTool()))
            .build();

        try (TypedModel model =
                 TypedModel.openaiWithBase("sk-test-fake-key", "gpt-4o", server.baseUrl())) {
            List<Types.StreamPart> parts = assertTimeout(Duration.ofSeconds(30),
                () -> model.streamTextStream("What is the weather in Tokyo?", options)
                    .collect(Collectors.toList()));

            assertThat(parts).isNotEmpty();
            // The stream must surface tool-call activity: either a complete
            // ToolCall part or ToolInputDelta deltas (or both).
            boolean hasToolPart = parts.stream().anyMatch(
                p -> p instanceof Types.StreamPart.ToolCall
                    || p instanceof Types.StreamPart.ToolInputDelta);
            assertThat(hasToolPart)
                .as("expected a ToolCall or ToolInputDelta stream part; got: %s", parts)
                .isTrue();

            // The finish part should also be present, carrying the tool-calls
            // finish reason.
            boolean hasFinish = parts.stream().anyMatch(p -> p instanceof Types.StreamPart.Finish);
            assertThat(hasFinish).isTrue();
        }
    }

    // ── Coverage gaps from review: callback stream, of(Model), List overload ──

    @Test
    void streamTextCallbackDeliversTypedParts() {
        server.setContentType("text/event-stream");
        server.setResponseBody(toolCallSseResponse());

        Types.GenerateTextOptions options = Types.GenerateTextOptions.builder()
            .tools(Collections.singletonList(weatherTool()))
            .build();

        java.util.List<Types.StreamPart> collected = new java.util.concurrent.CopyOnWriteArrayList<>();
        java.util.concurrent.atomic.AtomicBoolean done = new java.util.concurrent.atomic.AtomicBoolean(false);

        try (TypedModel model =
                 TypedModel.openaiWithBase("sk-test-fake-key", "gpt-4o", server.baseUrl())) {
            assertTimeout(Duration.ofSeconds(30), () ->
                model.streamText("What is the weather in Tokyo?", options,
                    collected::add,           // onPart — typed StreamPart
                    () -> done.set(true),     // onDone
                    err -> { throw new RuntimeException(err); }));
        }

        assertThat(collected).isNotEmpty();
        assertThat(done.get()).isTrue();
        // Callback delivers typed parts (not raw JSON strings).
        assertThat(collected.stream().anyMatch(
            p -> p instanceof Types.StreamPart.ToolCall
                || p instanceof Types.StreamPart.ToolInputDelta)).isTrue();
    }

    @Test
    void ofModelDoesNotOwnHandle() {
        server.setResponseBody(plainOpenAiResponse());

        Model raw = Model.openaiWithBase("sk-test-fake-key", "gpt-4o", server.baseUrl());
        // of() wraps without owning — closing the TypedModel must NOT close the raw Model.
        TypedModel typed = TypedModel.of(raw);
        typed.close();

        // The raw model is still usable because of() did not drop the handle.
        Types.GenerateTextResult result = typed.generateText("Still alive?");
        assertThat(result.getText()).isEqualTo("Rust is a systems programming language.");
        raw.close();
    }

    @Test
    void streamTextStreamFromMessageList() {
        server.setContentType("text/event-stream");
        server.setResponseBody(toolCallSseResponse());

        List<Types.ModelMessage> messages = Arrays.asList(
            Types.ModelMessage.text(Types.Role.USER, "What is the weather in Tokyo?"));

        Types.GenerateTextOptions options = Types.GenerateTextOptions.builder()
            .tools(Collections.singletonList(weatherTool()))
            .build();

        try (TypedModel model =
                 TypedModel.openaiWithBase("sk-test-fake-key", "gpt-4o", server.baseUrl())) {
            List<Types.StreamPart> parts = assertTimeout(Duration.ofSeconds(30),
                () -> model.streamTextStream(messages, options).collect(Collectors.toList()));

            assertThat(parts).isNotEmpty();
            assertThat(parts.stream().anyMatch(p -> p instanceof Types.StreamPart.Finish)).isTrue();
        }
    }

    // ── Coverage gap from review #2: TypedModel error paths ─────────────────

    @Test
    void generateTextInvalidPromptThrowsAimuxException() {
        // The typed layer JSON-encodes the prompt, so any string is valid wire
        // JSON; the failure is the mock provider's response → AimuxException.
        try (TypedModel model =
                 TypedModel.openaiWithBase("sk-test-fake-key", "gpt-4o", server.baseUrl())) {
            assertThatThrownBy(() -> model.generateText("not-valid-json"))
                .isInstanceOf(AimuxException.class);
        }
    }

    // Note: mid-stream provider errors are not deterministically triggerable
    // via a single-response mock; terminal stream failures throw AimuxException
    // from the C return/err path (same as raw Model.streamTextStream).
}
