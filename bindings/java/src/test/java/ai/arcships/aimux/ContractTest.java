package ai.arcships.aimux;

import com.fasterxml.jackson.databind.JsonNode;
import org.junit.jupiter.api.Test;

import java.io.File;
import java.io.IOException;
import java.util.Arrays;

import static org.assertj.core.api.Assertions.assertThat;

/**
 * Contract tests — validate the Java binding's wire-format consistency against the
 * shared fixtures in {@code contract-tests/fixtures/wire-format.json} (the same
 * fixtures used by the Rust contract tests and the Node runner).
 *
 * <p>Two deliberate deviations from a strict "deserialize → re-serialize == fixture"
 * round-trip, documented here because they reflect the binding's real behavior:
 *
 * <ol>
 *   <li><b>Null-field omission.</b> {@link Types.AimuxJson} uses
 *       {@code NON_NULL} inclusion (mirroring the Kotlin binding's intent of
 *       not emitting unset fields): only {@code null} fields are omitted on
 *       encode. {@code stream_part_stream_start} carries {@code "warnings": []},
 *       an empty (non-null) list, so re-serialization keeps it — the round-trip
 *       is exact.</li>
 *   <li><b>Finish's {@code {"unified":{"stop":null}}}.</b> The
 *       {@code stream_part_finish} fixture encodes the finish reason as an
 *       externally-tagged object, but the actual Rust wire format for
 *       {@code FinishReasonUnified} is the bare string {@code "stop"} (see
 *       {@code aimux-core/tests/contract_test.rs::finish_reason_unified_wire_format}),
 *       and the Java enum mirrors that string form. The fixture therefore cannot
 *       be deserialized by <em>any</em> binding (Rust serde would reject the
 *       object form too). This test asserts the fixture's JSON shape at the
 *       wire level and separately proves the Java {@link Types.StreamPart.Finish}
 *       round-trips with the binding's own {@code "unified":"stop"} encoding.</li>
 * </ol>
 *
 * <p>These tests are pure serialization tests: no native library is loaded.
 */
class ContractTest {

    /** Candidate locations for the shared fixtures, relative to the test working dir. */
    private static final String[] FIXTURE_CANDIDATES = {
        "../../contract-tests/fixtures/wire-format.json", // gradle test cwd = bindings/java
        "contract-tests/fixtures/wire-format.json",       // run from the repo root
        "../contract-tests/fixtures/wire-format.json",
    };

    // ── fixture loading ──────────────────────────────────────────────────────

    private static JsonNode loadFixtures() throws IOException {
        for (String candidate : FIXTURE_CANDIDATES) {
            File f = new File(candidate);
            if (f.isFile()) {
                return Types.AimuxJson.MAPPER.readTree(f);
            }
        }
        throw new IllegalStateException(
            "cannot find wire-format.json; tried " + Arrays.toString(FIXTURE_CANDIDATES)
                + " from " + new File(".").getAbsolutePath());
    }

    private static JsonNode fixture(JsonNode fixtures, String name) {
        for (JsonNode f : fixtures) {
            if (name.equals(f.path("name").asText())) {
                return f;
            }
        }
        throw new IllegalStateException("no fixture named '" + name + "'");
    }

    private static String fixtureJson(JsonNode fixtures, String name) {
        return fixture(fixtures, name).get("json").asText();
    }

    /** Semantic JSON equality (ignores key order / whitespace). */
    private static boolean jsonEquals(String a, String b) throws IOException {
        return Types.AimuxJson.MAPPER.readTree(a).equals(Types.AimuxJson.MAPPER.readTree(b));
    }

    // ── ToolChoice: round-trip + semantic type assertions ───────────────────

    @Test
    void toolChoiceFixturesRoundTripAndTypes() throws Exception {
        JsonNode fixtures = loadFixtures();

        // tool_choice_auto → "auto"
        String autoJson = fixtureJson(fixtures, "tool_choice_auto");
        Types.ToolChoice auto = Types.AimuxJson.MAPPER.readValue(autoJson, Types.ToolChoice.class);
        assertThat(auto).isInstanceOf(Types.ToolChoice.Auto.class);
        String autoOut = Types.AimuxJson.MAPPER.writeValueAsString(auto);
        assertThat(autoOut).isEqualTo(autoJson);

        // tool_choice_none → "none"
        String noneJson = fixtureJson(fixtures, "tool_choice_none");
        Types.ToolChoice none = Types.AimuxJson.MAPPER.readValue(noneJson, Types.ToolChoice.class);
        assertThat(none).isInstanceOf(Types.ToolChoice.None.class);
        assertThat(Types.AimuxJson.MAPPER.writeValueAsString(none)).isEqualTo(noneJson);

        // tool_choice_required → "required"
        String requiredJson = fixtureJson(fixtures, "tool_choice_required");
        Types.ToolChoice required = Types.AimuxJson.MAPPER.readValue(requiredJson, Types.ToolChoice.class);
        assertThat(required).isInstanceOf(Types.ToolChoice.Required.class);
        assertThat(Types.AimuxJson.MAPPER.writeValueAsString(required)).isEqualTo(requiredJson);

        // tool_choice_tool → {"type":"tool","toolName":"get_weather"}
        String toolJson = fixtureJson(fixtures, "tool_choice_tool");
        Types.ToolChoice tool = Types.AimuxJson.MAPPER.readValue(toolJson, Types.ToolChoice.class);
        assertThat(tool).isInstanceOf(Types.ToolChoice.Tool.class);
        assertThat(((Types.ToolChoice.Tool) tool).getToolName()).isEqualTo("get_weather");
        String toolOut = Types.AimuxJson.MAPPER.writeValueAsString(tool);
        assertThat(toolOut).isEqualTo(toolJson);
    }

    // ── StreamPart: round-trip + semantic type assertions ───────────────────

    @Test
    void streamPartTextDeltaRoundTripAndType() throws Exception {
        JsonNode fixtures = loadFixtures();
        String json = fixtureJson(fixtures, "stream_part_text_delta");

        Types.StreamPart part = Types.AimuxJson.MAPPER.readValue(json, Types.StreamPart.class);
        assertThat(part).isInstanceOf(Types.StreamPart.TextDelta.class);
        Types.StreamPart.TextDelta delta = (Types.StreamPart.TextDelta) part;
        assertThat(delta.getId()).isEqualTo("tx1");
        assertThat(delta.getDelta()).isEqualTo("Hello");

        // Exact round-trip: NON_NULL only omits the (null) provider_metadata.
        String out = Types.AimuxJson.MAPPER.writeValueAsString(part);
        assertThat(out).isEqualTo(json);
    }

    @Test
    void streamPartStreamStartRoundTripAndType() throws Exception {
        JsonNode fixtures = loadFixtures();
        String json = fixtureJson(fixtures, "stream_part_stream_start");

        Types.StreamPart part = Types.AimuxJson.MAPPER.readValue(json, Types.StreamPart.class);
        assertThat(part).isInstanceOf(Types.StreamPart.StreamStart.class);
        Types.StreamPart.StreamStart start = (Types.StreamPart.StreamStart) part;
        assertThat(start.getWarnings()).isEmpty();
        // warnings is an empty (non-null) list; NON_NULL inclusion keeps it,
        // so the re-encoded form retains the `warnings` key — exact round-trip.
        assertThat(Types.AimuxJson.MAPPER.writeValueAsString(part))
            .isEqualTo("{\"StreamStart\":{\"warnings\":[]}}");
    }

    /// RFC-0016 M2: the shared `stream_part_raw` fixture decodes to the Java
    /// `Raw` variant and round-trips exactly.
    @Test
    void streamPartRawRoundTripAndType() throws Exception {
        JsonNode fixtures = loadFixtures();
        String json = fixtureJson(fixtures, "stream_part_raw");

        Types.StreamPart part = Types.AimuxJson.MAPPER.readValue(json, Types.StreamPart.class);
        assertThat(part).isInstanceOf(Types.StreamPart.Raw.class);
        Types.StreamPart.Raw raw = (Types.StreamPart.Raw) part;
        assertThat(raw.getRawValue()).isNotNull();
        assertThat(raw.getRawValue().path("id").asText()).isEqualTo("c1");
        assertThat(raw.getRawValue().path("choices").isArray()).isTrue();

        // Exact round-trip (NON_NULL: raw_value is the only inner field).
        assertThat(Types.AimuxJson.MAPPER.writeValueAsString(part)).isEqualTo(json);
    }

    /// RFC-0016 M10: `Usage.raw` with a vendor-specific field survives a
    /// Java round-trip (NON_NULL omits a null raw, keeps a non-null one).
    @Test
    void usageRawPreservesVendorFieldsRoundTrip() throws Exception {
        Types.Usage usage = Types.Usage.builder()
            .inputTokens(Types.TokenUsage.builder().total(20L).build())
            .outputTokens(Types.TokenUsage.builder().total(5L).build())
            .raw(Types.AimuxJson.MAPPER.readTree(
                "{\"prompt_cache_hit_tokens\":42,\"prompt_tokens\":20}"))
            .build();

        String json = Types.AimuxJson.MAPPER.writeValueAsString(usage);
        assertThat(json).contains("\"raw\":{\"prompt_cache_hit_tokens\":42");
        Types.Usage decoded = Types.AimuxJson.MAPPER.readValue(json, Types.Usage.class);
        assertThat(decoded.getRaw()).isNotNull();
        assertThat(decoded.getRaw().path("prompt_cache_hit_tokens").asInt()).isEqualTo(42);
        assertThat(decoded.getRaw().path("prompt_tokens").asInt()).isEqualTo(20);
    }

    /// RFC-0024: `session_id` crosses the wire as snake_case and round-trips.
    @Test
    void generateTextOptionsSessionIdWireAndRoundTrip() throws Exception {
        // Wire shape: the shared fixture.
        JsonNode fixtures = loadFixtures();
        String json = fixtureJson(fixtures, "generate_text_options_with_session_id");
        Types.GenerateTextOptions opts =
            Types.AimuxJson.MAPPER.readValue(json, Types.GenerateTextOptions.class);
        assertThat(opts.getSessionId()).isEqualTo("sess-1");

        // Java-native round-trip with the Builder.
        Types.GenerateTextOptions nativeOpts = Types.GenerateTextOptions.builder()
            .sessionId("sess-1")
            .build();
        String nativeJson = Types.AimuxJson.MAPPER.writeValueAsString(nativeOpts);
        assertThat(nativeJson).contains("\"session_id\":\"sess-1\"");
        Types.GenerateTextOptions decoded =
            Types.AimuxJson.MAPPER.readValue(nativeJson, Types.GenerateTextOptions.class);
        assertThat(decoded.getSessionId()).isEqualTo("sess-1");
    }

    @Test
    void streamPartFinishWireShapeAndJavaRoundTrip() throws Exception {
        JsonNode fixtures = loadFixtures();
        String json = fixtureJson(fixtures, "stream_part_finish");

        // 1) Wire-level shape (as run-node.ts asserts): single "Finish" tag, usage totals.
        JsonNode finishNode = Types.AimuxJson.MAPPER.readTree(json);
        assertThat(finishNode.size()).isEqualTo(1);
        JsonNode inner = finishNode.get("Finish");
        assertThat(inner).isNotNull();
        assertThat(inner.path("finish_reason").path("unified").path("stop").isNull()).isTrue();
        assertThat(inner.path("usage").path("input_tokens").path("total").asInt()).isEqualTo(10);
        assertThat(inner.path("usage").path("output_tokens").path("total").asInt()).isEqualTo(20);
        assertThat(inner.path("provider_metadata").isNull()).isTrue();

        // 2) The Java binding cannot deserialize the fixture's object-form enum
        //    ({"unified":{"stop":null}}); its FinishReasonUnified is the string
        //    wire format "stop" — the same as Rust's actual serialization.
        try {
            Types.AimuxJson.MAPPER.readValue(json, Types.StreamPart.class);
            throw new AssertionError("expected MismatchedInputException for object-form enum");
        } catch (com.fasterxml.jackson.databind.exc.MismatchedInputException expected) {
            // expected: the fixture's externally-tagged enum form is not the wire format
        }

        // 3) Java-native Finish round-trips with the binding's own encoding.
        Types.StreamPart.Finish nativeFinish = Types.StreamPart.Finish.builder()
            .finishReason(Types.FinishReason.builder().unified(Types.FinishReasonUnified.STOP).build())
            .usage(Types.Usage.of(10L, 20L))
            .build();
        String nativeJson = Types.AimuxJson.MAPPER.writeValueAsString(nativeFinish);
        // NON_NULL omits the null raw / provider_metadata fields.
        assertThat(nativeJson).isEqualTo(
            "{\"Finish\":{\"finish_reason\":{\"unified\":\"stop\"},"
                + "\"usage\":{\"input_tokens\":{\"total\":10},\"output_tokens\":{\"total\":20}}}}");
        Types.StreamPart decoded = Types.AimuxJson.MAPPER.readValue(nativeJson, Types.StreamPart.class);
        assertThat(decoded).isInstanceOf(Types.StreamPart.Finish.class);
        Types.StreamPart.Finish f = (Types.StreamPart.Finish) decoded;
        assertThat(f.getFinishReason().getUnified()).isEqualTo(Types.FinishReasonUnified.STOP);
        assertThat(f.getUsage().getInputTokens().getTotal()).isEqualTo(10L);
        assertThat(f.getUsage().getOutputTokens().getTotal()).isEqualTo(20L);
    }

    // ── sanity: every fixture parses as valid JSON (mirrors the Rust contract test) ──

    @Test
    void allFixturesAreValidJson() throws Exception {
        JsonNode fixtures = loadFixtures();
        assertThat(fixtures.isArray() && fixtures.size() > 0).isTrue();
        for (JsonNode f : fixtures) {
            Types.AimuxJson.MAPPER.readTree(f.get("json").asText());
        }
    }
}
