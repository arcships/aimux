package ai.arcships.aimux;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import org.junit.jupiter.api.Test;

import static org.assertj.core.api.Assertions.assertThat;

/** The payload's {@code cause} chain becomes the exception's {@code getCause()} chain. */
class ErrorCauseTest {
    private static final ObjectMapper MAPPER = new ObjectMapper();

    private static NativeException decode(String error) throws Exception {
        JsonNode envelope = MAPPER.readTree(
            "{\"kind\":\"aimux\",\"code\":11,\"message\":\"API call error: Cannot connect\",\"error\":" + error + "}");
        return AimuxResult.decode(envelope);
    }

    @Test void transportCauseChainIsTheCauseChain() throws Exception {
        NativeException error = decode("{\"name\":\"AI_APICallError\",\"message\":\"Cannot connect\","
            + "\"cause\":{\"message\":\"tcp connect error\",\"cause\":{\"message\":\"Connection refused\"}}}");
        assertThat(error).isInstanceOf(AimuxException.APICallError.class)
            .hasCauseInstanceOf(ErrorCause.class)
            .hasRootCauseMessage("Connection refused");
        ErrorCause cause = (ErrorCause) error.getCause();
        assertThat(cause.getMessage()).isEqualTo("tcp connect error");
        assertThat(cause.getErrorName()).isEmpty();
    }

    @Test void nestedAimuxCauseKeepsItsName() throws Exception {
        NativeException error = decode("{\"name\":\"AI_InvalidToolInputError\",\"toolName\":\"t\",\"toolInput\":\"{\","
            + "\"cause\":{\"name\":\"AI_JSONParseError\",\"message\":\"JSON parsing failed\"}}");
        ErrorCause cause = (ErrorCause) error.getCause();
        assertThat(cause.getErrorName()).isEqualTo("AI_JSONParseError");
        assertThat(cause.getError().path("message").asText()).isEqualTo("JSON parsing failed");
        assertThat(cause.getCause()).isNull();
    }

    @Test void noCauseLeavesTheCauseFree() throws Exception {
        NativeException error = decode("{\"name\":\"TimeoutError\",\"message\":\"t\"}");
        assertThat(error.getCause()).isNull();
        assertThat(AimuxResult.callbackFailure(new IllegalStateException("x")).getCause())
            .isInstanceOf(IllegalStateException.class);
    }
}
