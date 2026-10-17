package ai.arcships.aimux;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;

/** Test access to Java's decoder; production Kotlin never decodes envelopes. */
public final class KotlinInteropFixtures {
    private KotlinInteropFixtures() {}

    public static NativeException decode(String json) throws Exception {
        return AimuxResult.decode(new ObjectMapper().readTree(json));
    }

    public static NativeException unknown(String json) throws Exception {
        return new FutureError(new ObjectMapper().readTree(json));
    }

    private static final class FutureError extends NativeException {
        FutureError(JsonNode envelope) { super(envelope); }
    }

    public static Model acceptModel(Model value) { return value; }
    public static ProviderHandle acceptProvider(ProviderHandle value) { return value; }
}
