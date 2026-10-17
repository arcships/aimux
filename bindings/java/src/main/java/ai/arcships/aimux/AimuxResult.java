package ai.arcships.aimux;

import com.fasterxml.jackson.databind.DeserializationFeature;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import com.sun.jna.Pointer;
import com.sun.jna.ptr.PointerByReference;
import java.io.IOException;
import java.util.Objects;

final class AimuxResult {
    private AimuxResult() {}

    static JsonNode parse(String value) {
        Objects.requireNonNull(value, "JSON");
        try {
            JsonNode node = Types.AimuxJson.MAPPER.reader()
                .with(DeserializationFeature.FAIL_ON_TRAILING_TOKENS).readTree(value);
            if (node == null || node.isMissingNode()) throw new IllegalArgumentException("Empty JSON");
            return node;
        } catch (IOException e) {
            throw new IllegalArgumentException("Invalid JSON", e);
        }
    }

    static String params(String json) {
        if (!parse(json).isObject()) throw new IllegalArgumentException("params must be a JSON object");
        return json;
    }

    static String takeString(AimuxFFI ffi, Pointer pointer) {
        if (pointer == null) return null;
        try { return pointer.getString(0, "UTF-8"); }
        finally { ffi.aimux_free_string(pointer); }
    }

    static JsonNode result(AimuxFFI ffi, Pointer error, PointerByReference out) {
        try {
            check(ffi, error);
            Pointer pointer = out.getValue();
            out.setValue(null);
            String json = takeString(ffi, pointer);
            if (json == null) throw new IllegalStateException("Native success has no JSON result");
            return parse(json);
        } finally {
            if (out.getValue() != null) ffi.aimux_free_string(out.getValue());
        }
    }

    static void check(AimuxFFI ffi, Pointer error) {
        if (error == null) return;
        try {
            int code = ffi.aimux_error_code(error);
            JsonNode envelope = parse(takeString(ffi, ffi.aimux_error_json(error)));
            if (envelope.path("code").asInt() != code) {
                throw new IllegalStateException("Native error code/envelope mismatch");
            }
            throw decode(envelope);
        } finally { ffi.aimux_error_free(error); }
    }

    static NativeException decode(JsonNode envelope) {
        if (!envelope.path("code").isInt() || !envelope.path("message").isTextual()
            || !envelope.path("error").path("name").isTextual()) {
            throw new IllegalStateException("Malformed native error envelope");
        }
        switch (envelope.path("kind").asText()) {
            case "aimux": return AimuxException.decode(envelope);
            case "recording": return RecordingException.decode(envelope);
            case "binding": return BindingException.decode(envelope);
            case "protocol": return ProtocolException.decode(envelope);
            default: throw new IllegalStateException("Unknown native error kind");
        }
    }

    static BindingException.CallbackFailure callbackFailure(Throwable cause) {
        ObjectNode envelope = Types.AimuxJson.MAPPER.createObjectNode();
        envelope.put("kind", "binding");
        envelope.put("code", 206);
        envelope.put("message", "Stream consumer failed");
        envelope.putObject("error").put("name", "CallbackFailure");
        BindingException.CallbackFailure failure = new BindingException.CallbackFailure(envelope);
        failure.initCause(cause);
        return failure;
    }

    static long handle(JsonNode value) {
        // Handles are decimal strings, never JSON numbers (including values above 2^53).
        JsonNode id = value.isObject() ? value.path("handle") : value;
        if (!id.isTextual() || !id.textValue().matches("[1-9][0-9]*")) {
            throw new IllegalStateException("Expected a nonzero decimal handle string");
        }
        try { return Long.parseUnsignedLong(id.textValue()); }
        catch (NumberFormatException e) { throw new IllegalStateException("Handle exceeds uint64", e); }
    }
}
