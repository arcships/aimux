package ai.arcships.aimux;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import org.json.JSONArray;
import org.json.JSONObject;
import org.junit.jupiter.api.Test;

import java.io.File;
import java.io.IOException;
import java.time.Duration;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;
import java.util.stream.Collectors;

import static org.assertj.core.api.Assertions.assertThat;
import static org.assertj.core.api.Assertions.assertThatThrownBy;
import static org.junit.jupiter.api.Assertions.assertTimeoutPreemptively;

/**
 * Host-side tool-call repair (RFC-0035).
 *
 * <p>Two layers: a replay of the shared
 * {@code contract-tests/fixtures/tool-call-repair.json} over the three native
 * functions, plus public typed non-streaming/streaming integration and the
 * same-model nested-generation deadlock regression. Tests use only mock
 * providers, but need the native library on {@code java.library.path}.
 */
class ToolCallRepairTest {

    private static final String[] FIXTURE_CANDIDATES = {
        "../../contract-tests/fixtures/tool-call-repair.json", // gradle test cwd = bindings/java
        "contract-tests/fixtures/tool-call-repair.json",       // run from the repo root
        "../contract-tests/fixtures/tool-call-repair.json",
    };

    // ── shared fixture replay ────────────────────────────────────────────────

    @Test
    void contractFixtureReplays() throws Exception {
        JsonNode cases = loadFixture().get("cases");
        List<String> replayed = new ArrayList<>();
        for (JsonNode c : cases) {
            String name = c.get("name").asText();
            String expectedError = c.path("expected_error").asText(null);
            if (expectedError != null) {
                // Every error case in the fixture is InvalidArgument; if a new
                // variant appears, this pins it rather than silently passing.
                assertThat(expectedError).describedAs(name).isEqualTo("InvalidArgument");
                assertThatThrownBy(() -> invoke(c))
                    .describedAs(name)
                    .isInstanceOf(AimuxException.InvalidArgumentError.class);
            } else {
                assertThat(json(invoke(c))).describedAs(name).isEqualTo(c.get("expected"));
            }
            replayed.add(name);
        }
        assertThat(replayed).hasSize(cases.size());
    }

    private static String invoke(JsonNode testCase) {
        JsonNode in = testCase.get("input");
        String function = testCase.get("function").asText();
        String opts = text(in.get("opts"));
        if ("tool_call_repair_context".equals(function)) {
            return ToolCallRepairs.context(in.get("tool_call").toString(), text(in.get("prompt")), opts);
        }
        if ("apply_tool_call_repair".equals(function)) {
            return ToolCallRepairs.apply(in.get("tool_call").toString(), opts, in.get("reply").toString());
        }
        if ("apply_tool_call_repair_to_result".equals(function)) {
            return ToolCallRepairs.applyToResult(in.get("result").toString(), opts,
                in.get("tool_call_id").asText(), in.get("reply").toString());
        }
        throw new AssertionError("unknown fixture function: " + function);
    }

    // ── end to end through TypedModel (mock provider) ────────────────────────

    /** The strict tool wants {@code location}; the canned calls send {@code city}. */
    private static Types.RawToolCall cityToLocation(Types.ToolCallRepairContext context) {
        return Types.RawToolCall.builder()
            .toolCallId(context.getToolCall().getToolCallId())
            .toolName(context.getToolCall().getToolName())
            .input(context.getToolCall().getInput().replace("city", "location"))
            .build();
    }

    @Test
    void generateTextRepairsTheInvalidCallItGotBack() {
        try (MockProviderServer server = new MockProviderServer()) {
            server.setResponseBody(toolCallResponse("{\"city\":\"Tokyo\"}"));
            Types.GenerateTextOptions options = Types.GenerateTextOptions.builder()
                .tools(Collections.singletonList(strictWeatherTool()))
                .repairToolCall(context -> {
                    assertThat(context.getToolCall().getInput()).isEqualTo("{\"city\":\"Tokyo\"}");
                    return cityToLocation(context);
                })
                .build();

            try (TypedModel model =
                     TypedModel.openaiWithBase("sk-test-fake-key", "gpt-4o", server.baseUrl())) {
                Types.GenerateTextResult result =
                    model.generateText("What is the weather in Tokyo?", options);

                Types.ToolCall call = result.getToolCalls().get(0);
                assertThat(call.getInvalid()).isNull();
                assertThat(call.getInput().get("location").asText()).isEqualTo("Tokyo");
                Types.ContentPart.ToolCall transcriptCall = result.getResponseMessages().stream()
                    .flatMap(message -> message.getContentParts().stream())
                    .filter(part -> part instanceof Types.ContentPart.ToolCall)
                    .map(part -> (Types.ContentPart.ToolCall) part)
                    .findFirst().orElseThrow(AssertionError::new);
                assertThat(transcriptCall.getInput().get("location").asText()).isEqualTo("Tokyo");
            }
        }
    }

    @Test
    void generateTextAsOpenAICarriesTheRepairedArguments() {
        try (MockProviderServer server = new MockProviderServer()) {
            server.setResponseBody(toolCallResponse("{\"city\":\"Tokyo\"}"));
            Types.GenerateTextOptions options = Types.GenerateTextOptions.builder()
                .tools(Collections.singletonList(strictWeatherTool()))
                .repairToolCall(ToolCallRepairTest::cityToLocation)
                .build();

            try (TypedModel model =
                     TypedModel.openaiWithBase("sk-test-fake-key", "gpt-4o", server.baseUrl())) {
                // A ChatCompletion has no invalid marker: the native result is
                // repaired, then converted.
                Types.ChatCompletion completion =
                    model.generateTextAsOpenAI("What is the weather in Tokyo?", options);

                Types.ChatCompletionToolCall call =
                    completion.getChoices().get(0).getMessage().getToolCalls().get(0);
                assertThat(call.getFunction().getArguments()).isEqualTo("{\"location\":\"Tokyo\"}");
            }
        }
    }

    @Test
    void aStreamRepairFailingAtTheBoundaryStillDeliversTheCall() {
        try (MockProviderServer server = new MockProviderServer()) {
            server.setContentType("text/event-stream");
            server.setResponseBody(toolCallSse("{\"city\":\"Tokyo\"}"));
            // A replacement without a tool_call_id is not a RawToolCall on the
            // wire: the library rejects the reply itself (not the hook failing).
            Types.GenerateTextOptions options = Types.GenerateTextOptions.builder()
                .tools(Collections.singletonList(strictWeatherTool()))
                .repairToolCall(context -> Types.RawToolCall.builder()
                    .toolCallId(null)
                    .toolName(context.getToolCall().getToolName())
                    .input("{\"location\":\"Tokyo\"}")
                    .build())
                .build();

            try (TypedModel model =
                     TypedModel.openaiWithBase("sk-test-fake-key", "gpt-4o", server.baseUrl())) {
                List<Types.StreamPart> parts = new ArrayList<>();
                List<String> errors = new ArrayList<>();
                model.streamText("What is the weather in Tokyo?", options,
                    parts::add, () -> { }, errors::add);

                assertThat(errors).hasSize(1);
                assertThat(errors.get(0)).startsWith("failed to repair tool call:");
                List<Types.StreamPart.ToolCall> calls = parts.stream()
                    .filter(p -> p instanceof Types.StreamPart.ToolCall)
                    .map(p -> (Types.StreamPart.ToolCall) p)
                    .collect(Collectors.toList());
                assertThat(calls).hasSize(1);
                assertThat(calls.get(0).getInvalid()).isTrue();
            }
        }
    }

    @Test
    void streamTextStreamDeliversTheRepairedToolCallPart() {
        try (MockProviderServer server = new MockProviderServer()) {
            server.setContentType("text/event-stream");
            server.setResponseBody(toolCallSse("{\"city\":\"Tokyo\"}"));
            Types.GenerateTextOptions options = Types.GenerateTextOptions.builder()
                .tools(Collections.singletonList(strictWeatherTool()))
                .repairToolCall(ToolCallRepairTest::cityToLocation)
                .build();

            try (TypedModel model =
                     TypedModel.openaiWithBase("sk-test-fake-key", "gpt-4o", server.baseUrl())) {
                List<Types.StreamPart> parts =
                    model.streamTextStream("What is the weather in Tokyo?", options)
                        .collect(Collectors.toList());

                List<Types.StreamPart.ToolCall> calls = parts.stream()
                    .filter(p -> p instanceof Types.StreamPart.ToolCall)
                    .map(p -> (Types.StreamPart.ToolCall) p)
                    .collect(Collectors.toList());
                assertThat(calls).hasSize(1);
                assertThat(calls.get(0).getInvalid()).isNull();
                assertThat(calls.get(0).getInput().get("location").asText()).isEqualTo("Tokyo");

                // Argument deltas are forwarded verbatim, repair or not.
                assertThat(parts.stream()
                    .filter(p -> p instanceof Types.StreamPart.ToolInputDelta)
                    .map(p -> ((Types.StreamPart.ToolInputDelta) p).getDelta())
                    .collect(Collectors.joining())).contains("city");
            }
        }
    }

    @Test
    void theRepairLoopHandlesThrowNullValidCallAndNoToolSet() {
        try (MockProviderServer server = new MockProviderServer();
             TypedModel model =
                 TypedModel.openaiWithBase("sk-test-fake-key", "gpt-4o", server.baseUrl())) {
            final AtomicInteger hookCalls = new AtomicInteger();
            ToolCallRepair countingNull = context -> {
                hookCalls.incrementAndGet();
                return null;
            };

            // The hook throws: ToolCallRepair { original_error, cause: Other }.
            server.setResponseBody(toolCallResponse("{\"city\":\"Tokyo\"}"));
            Types.ToolCall call = model.generateText("What is the weather in Tokyo?",
                Types.GenerateTextOptions.builder()
                    .tools(Collections.singletonList(strictWeatherTool()))
                    .repairToolCall(context -> {
                        hookCalls.incrementAndGet();
                        throw new IllegalStateException("repair model unavailable");
                    })
                    .build()).getToolCalls().get(0);
            assertThat(call.getInvalid()).isTrue();
            JsonNode failure = call.getError().path("ToolCallRepair");
            assertThat(failure.path("cause").path("Other").asText()).isEqualTo("repair model unavailable");
            assertThat(failure.path("original_error").has("InvalidToolInput")).isTrue();
            assertThat(hookCalls.get()).isEqualTo(1);

            // The hook returns null: the ORIGINAL error, not wrapped in ToolCallRepair.
            server.setResponseBody(toolCallResponse("{\"city\":\"Tokyo\"}"));
            call = model.generateText("What is the weather in Tokyo?",
                Types.GenerateTextOptions.builder()
                    .tools(Collections.singletonList(strictWeatherTool()))
                    .repairToolCall(countingNull)
                    .build()).getToolCalls().get(0);
            assertThat(call.getInvalid()).isTrue();
            assertThat(call.getError().has("ToolCallRepair")).isFalse();
            assertThat(call.getError().has("InvalidToolInput")).isTrue();
            assertThat(hookCalls.get()).isEqualTo(2);

            // A valid call never reaches the hook.
            server.setResponseBody(toolCallResponse("{\"location\":\"Tokyo\"}"));
            call = model.generateText("What is the weather in Tokyo?",
                Types.GenerateTextOptions.builder()
                    .tools(Collections.singletonList(strictWeatherTool()))
                    .repairToolCall(countingNull)
                    .build()).getToolCalls().get(0);
            assertThat(call.getInvalid()).isNull();
            assertThat(hookCalls.get()).isEqualTo(2);

            // No tool set: the native context answers null, so the host skips the hook.
            server.setResponseBody(toolCallResponse("{\"city\":\"Tokyo\"}"));
            call = model.generateText("What is the weather in Tokyo?",
                Types.GenerateTextOptions.builder().repairToolCall(countingNull).build())
                .getToolCalls().get(0);
            assertThat(call.getInvalid()).isTrue();
            assertThat(hookCalls.get()).isEqualTo(2);
        }
    }

    @Test
    void aStreamHookMayGenerateWithTheSameModelWhileCloseIsPending() {
        // Regression: the stream holds Model's (fair) read lock for its whole
        // duration, a queued close() waits for it, and a hook generating with
        // the SAME model used to queue behind that writer — S waits for R waits
        // for C waits for S. The stream thread lends its read hold to the repair
        // worker so the hook's call takes no lock at all.
        assertTimeoutPreemptively(Duration.ofSeconds(30), () -> {
            try (MockProviderServer server = new MockProviderServer()) {
                server.setContentType("text/event-stream");
                server.setResponses(toolCallSse("{\"city\":\"Tokyo\"}"),
                                    textResponse("{\"location\":\"Tokyo\"}"));

                final Model raw =
                    Model.openaiWithBase("sk-test-fake-key", "gpt-4o", server.baseUrl());
                final TypedModel model = TypedModel.of(raw);
                final CountDownLatch hookStarted = new CountDownLatch(1);
                final AtomicReference<Throwable> closerFailure = new AtomicReference<>();
                Thread closer = new Thread(() -> {
                    try {
                        hookStarted.await();
                        raw.close();
                    } catch (Throwable t) {
                        closerFailure.set(t);
                    }
                }, "closer");
                closer.start();

                Types.GenerateTextOptions options = Types.GenerateTextOptions.builder()
                    .tools(Collections.singletonList(strictWeatherTool()))
                    .repairToolCall(context -> {
                        hookStarted.countDown();
                        // Let the closer actually enqueue for the write lock;
                        // without it the race the test reproduces may not happen.
                        Thread.sleep(200);
                        // The mock server's content type is global, so flip it
                        // now that the SSE response has already been written.
                        server.setContentType("application/json");
                        return Types.RawToolCall.builder()
                            .toolCallId(context.getToolCall().getToolCallId())
                            .toolName(context.getToolCall().getToolName())
                            .input(model.generateText(
                                "fix these arguments: " + context.getToolCall().getInput())
                                .getText())
                            .build();
                    })
                    .build();

                List<Types.StreamPart> parts = new ArrayList<>();
                model.streamText("What is the weather in Tokyo?", options,
                    parts::add, () -> {}, error -> { throw new AssertionError(error); });

                closer.join(10_000);
                assertThat(closer.isAlive()).isFalse();
                assertThat(closerFailure.get()).isNull();

                List<Types.StreamPart.ToolCall> calls = parts.stream()
                    .filter(p -> p instanceof Types.StreamPart.ToolCall)
                    .map(p -> (Types.StreamPart.ToolCall) p)
                    .collect(Collectors.toList());
                assertThat(calls).hasSize(1);
                assertThat(calls.get(0).getInvalid()).isNull();
                assertThat(calls.get(0).getInput().get("location").asText()).isEqualTo("Tokyo");
                raw.close(); // idempotent; the closer already did it
            }
        });
    }

    /** A plain OpenAI text response whose content is {@code text}. */
    private static String textResponse(String text) {
        return new JSONObject()
            .put("id", "chatcmpl-repair")
            .put("model", "gpt-4o")
            .put("choices", new JSONArray().put(new JSONObject()
                .put("message", new JSONObject().put("role", "assistant").put("content", text))
                .put("finish_reason", "stop")))
            .put("usage", new JSONObject()
                .put("prompt_tokens", 4).put("completion_tokens", 4).put("total_tokens", 8))
            .toString();
    }

    /** {@code get_weather}, schema-strict so a {@code city} argument is rejected. */
    private static Types.Tool strictWeatherTool() {
        ObjectNode schema = Types.AimuxJson.MAPPER.createObjectNode();
        schema.put("type", "object");
        schema.set("properties", Types.AimuxJson.MAPPER.createObjectNode()
            .set("location", Types.AimuxJson.MAPPER.createObjectNode().put("type", "string")));
        schema.set("required", Types.AimuxJson.MAPPER.createArrayNode().add("location"));
        schema.put("additionalProperties", false);
        return Types.Tool.Function.builder().name("get_weather").inputSchema(schema).build();
    }

    private static String toolCallResponse(String arguments) {
        return new JSONObject()
            .put("id", "chatcmpl-tc")
            .put("model", "gpt-4o")
            .put("choices", new JSONArray().put(new JSONObject()
                .put("message", new JSONObject()
                    .put("role", "assistant")
                    .put("content", JSONObject.NULL)
                    .put("tool_calls", new JSONArray().put(new JSONObject()
                        .put("id", "call_abc")
                        .put("type", "function")
                        .put("function", new JSONObject()
                            .put("name", "get_weather")
                            .put("arguments", arguments)))))
                .put("finish_reason", "tool_calls")))
            .put("usage", new JSONObject()
                .put("prompt_tokens", 20).put("completion_tokens", 10).put("total_tokens", 30))
            .toString();
    }

    private static String toolCallSse(String arguments) {
        StringBuilder sb = new StringBuilder();
        sb.append("data: ").append(new JSONObject()
            .put("id", "1").put("model", "gpt-4o")
            .put("choices", new JSONArray().put(new JSONObject().put("delta", new JSONObject()
                .put("role", "assistant")
                .put("tool_calls", new JSONArray().put(new JSONObject()
                    .put("index", 0).put("id", "call_xyz").put("type", "function")
                    .put("function", new JSONObject().put("name", "get_weather")
                        .put("arguments", "")))))))).append("\n\n");
        sb.append("data: ").append(new JSONObject()
            .put("id", "1").put("model", "gpt-4o")
            .put("choices", new JSONArray().put(new JSONObject().put("delta", new JSONObject()
                .put("tool_calls", new JSONArray().put(new JSONObject()
                    .put("index", 0)
                    .put("function", new JSONObject().put("arguments", arguments)))))))).append("\n\n");
        sb.append("data: ").append(new JSONObject()
            .put("id", "1").put("model", "gpt-4o")
            .put("choices", new JSONArray().put(new JSONObject()
                .put("delta", new JSONObject()).put("finish_reason", "tool_calls")))
            .put("usage", new JSONObject()
                .put("prompt_tokens", 5).put("completion_tokens", 2).put("total_tokens", 7)))
            .append("\n\n");
        sb.append("data: [DONE]\n\n");
        return sb.toString();
    }

    private static JsonNode json(String raw) throws IOException {
        return Types.AimuxJson.MAPPER.readTree(raw);
    }

    private static String text(JsonNode node) {
        return node == null || node.isNull() ? null : node.toString();
    }

    private static JsonNode loadFixture() throws IOException {
        for (String candidate : FIXTURE_CANDIDATES) {
            File f = new File(candidate);
            if (f.isFile()) {
                return Types.AimuxJson.MAPPER.readTree(f);
            }
        }
        throw new IllegalStateException(
            "cannot find tool-call-repair.json; tried " + Arrays.toString(FIXTURE_CANDIDATES)
                + " from " + new File(".").getAbsolutePath());
    }
}
