package ai.arcships.aimux;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.sun.net.httpserver.HttpServer;
import java.net.InetSocketAddress;
import java.nio.file.Paths;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class DecisionModelTest {
    @Test void providerDecisionModelOwnsItsHandle() throws Exception {
        ProviderHandle provider = Model.createProvider("openai", "test-key", null);
        try (DecisionModel model = provider.decisionModel("gpt-6-luna")) {
            provider.close();
            JsonNode description = new ObjectMapper().readTree(model.describe());
            assertEquals("v4", description.get("specificationVersion").asText());
            assertEquals("openai", description.get("provider").asText());
            assertEquals("gpt-6-luna", description.get("modelId").asText());
            assertThrows(IllegalStateException.class, () -> provider.decisionModel("gpt-6-luna"));
        } finally { provider.close(); }
    }

    @Test void officialStructuredContractAndDescription() throws Exception {
        ObjectMapper mapper = new ObjectMapper();
        JsonNode fixture = mapper.readTree(Paths.get("../../contract-tests/fixtures/decision-native.json").toFile());
        AtomicReference<JsonNode> captured = new AtomicReference<>();
        HttpServer server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        server.createContext("/v1/systemone", exchange -> {
            captured.set(mapper.readTree(exchange.getRequestBody()));
            byte[] body = fixture.get("response").toString().getBytes(StandardCharsets.UTF_8);
            exchange.getResponseHeaders().set("Content-Type", "application/json");
            exchange.sendResponseHeaders(200, body.length);
            exchange.getResponseBody().write(body);
            exchange.close();
        });
        server.start();
        try (DecisionModel model = DecisionModel.jev("test-key", "jev-latest",
                "http://127.0.0.1:" + server.getAddress().getPort() + "/v1/systemone", "native")) {
            JsonNode description = mapper.readTree(model.describe());
            assertEquals("jev", description.get("provider").asText());
            assertEquals(3, description.get("supportedQuestionTypes").size());
            assertNull(captured.get());
            JsonNode result = mapper.readTree(model.decide(fixture.get("request").toString()));
            assertEquals(0.9, result.get("answers").get("urgent").get("probability").asDouble());
            assertEquals(2, result.get("rounding").get("scoreDecimals").asInt());
            assertEquals(mapper.readTree("{\"inputTokens\":50,\"outputTokens\":10,\"totalTokens\":60}"),
                result.get("usage"));
            assertEquals(fixture.get("request").get("questions").get("urgent").get("criteria"),
                captured.get().get("questions").get("urgent").get("criteria"));
            assertEquals(fixture.get("request").get("questions").get("severity").get("criteria"),
                captured.get().get("questions").get("severity").get("criteria"));
            assertThrows(IllegalArgumentException.class, () -> model.decide("{"));
            model.close();
            assertThrows(IllegalStateException.class, model::describe);
        } finally { server.stop(0); }
        assertThrows(AimuxException.class, () -> DecisionModel.jev("test-key", "jev-latest", null, "unknown"));
    }

    @Test void openaiMediaAndTypedValues() throws Exception {
        ObjectMapper mapper = new ObjectMapper();
        JsonNode fixture = mapper.readTree(Paths.get("../../contract-tests/fixtures/decision-openai-full.json").toFile());
        AtomicReference<JsonNode> captured = new AtomicReference<>();
        HttpServer server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        server.createContext("/v1/decisions", exchange -> {
            captured.set(mapper.readTree(exchange.getRequestBody()));
            byte[] body = fixture.get("response").toString().getBytes(StandardCharsets.UTF_8);
            exchange.getResponseHeaders().set("Content-Type", "application/json");
            exchange.sendResponseHeaders(200, body.length);
            exchange.getResponseBody().write(body);
            exchange.close();
        });
        server.start();
        String config = mapper.createObjectNode().put("base_url", "http://127.0.0.1:" + server.getAddress().getPort() + "/v1").toString();
        try (ProviderHandle provider = Model.createProvider("openai", "test-key", config);
             DecisionModel model = provider.decisionModel("gpt-6-luna")) {
            JsonNode result = mapper.readTree(model.decide(fixture.get("options").toString()));
            assertEquals(fixture.get("request"), captured.get());
            assertEquals("boolean_true", result.get("answers").get("choice").get("choice").asText());
            JsonNode value = result.get("providerMetadata").get("openai").get("answers").get("choice").get("value");
            assertTrue(value.isBoolean());
            assertTrue(value.asBoolean());
            assertEquals(0.75, result.get("answers").get("score").get("score").asDouble());
        } finally { server.stop(0); }
    }
}
