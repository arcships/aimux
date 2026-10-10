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
            assertEquals(2, new ObjectMapper().readTree(model.capabilities()).get("min_choices").asInt());
            assertThrows(IllegalStateException.class, () -> provider.decisionModel("gpt-6-luna"));
        } finally { provider.close(); }
    }

    @Test void officialStructuredContractAndCapabilities() throws Exception {
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
            JsonNode caps = mapper.readTree(model.capabilities());
            assertEquals(255, caps.get("max_choices").asInt());
            assertEquals(2, caps.get("rounding").get("score_decimals").asInt());
            assertNull(captured.get());
            JsonNode result = mapper.readTree(model.decide(fixture.get("request").toString()));
            assertEquals(0.9, result.get("answers").get("urgent").get("probability_true").asDouble());
            assertEquals(fixture.get("request").get("questions").get(0).get("criteria"),
                captured.get().get("questions").get("urgent").get("criteria"));
            assertEquals(fixture.get("request").get("questions").get(2).get("levels"),
                result.get("answers").get("severity").get("levels"));
            assertThrows(IllegalArgumentException.class, () -> model.decide("{"));
            model.close();
            assertThrows(IllegalStateException.class, model::capabilities);
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
            assertTrue(result.get("answers").get("choice").get("value").isBoolean());
            assertTrue(result.get("answers").get("choice").get("value").asBoolean());
            assertEquals(fixture.get("options").get("questions").get(1).get("levels"),
                result.get("answers").get("score").get("levels"));
        } finally { server.stop(0); }
    }
}
