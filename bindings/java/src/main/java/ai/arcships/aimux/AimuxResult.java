package ai.arcships.aimux;

import ai.arcships.aimux.types.*;

import com.fasterxml.jackson.databind.JsonNode;
import com.sun.jna.Pointer;
import com.sun.jna.ptr.PointerByReference;
import java.util.Objects;

final class AimuxResult {
    private AimuxResult() {}

    static JsonNode parse(String value) {
        Objects.requireNonNull(value, "JSON");
        return WireJson.parse(value);
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
            ErrorEnvelope envelope = ErrorEnvelope.fromJson(parse(takeString(ffi, ffi.aimux_error_json(error))));
            if (envelope.getCode() != code) {
                throw new IllegalStateException("Native error code/envelope mismatch");
            }
            throw decode(envelope);
        } finally { ffi.aimux_error_free(error); }
    }

    static NativeException decode(JsonNode envelope) {
        try { return decode(ErrorEnvelope.fromJson(envelope)); }
        catch (IllegalArgumentException error) { throw new IllegalStateException("Malformed native error envelope", error); }
    }

    static NativeException decode(ErrorEnvelope envelope) {
        if (!envelope.getError().path("name").isTextual()) {
            throw new IllegalStateException("Malformed native error payload");
        }
        switch (envelope.getKind()) {
            case AIMUX: return AimuxException.decode(envelope);
            case RECORDING: return RecordingException.decode(envelope);
            case BINDING: return BindingException.decode(envelope);
            case PROTOCOL: return ProtocolException.decode(envelope);
            default: throw new IllegalStateException("Unknown native error kind");
        }
    }

    static BindingException.CallbackFailure callbackFailure(Throwable cause) {
        ErrorEnvelope envelope = ErrorEnvelope.builder().kind(ErrorDomain.BINDING).code(206)
            .message("Stream consumer failed")
            .error(WireJson.MAPPER.createObjectNode().put("name", "CallbackFailure")).build();
        BindingException.CallbackFailure failure = new BindingException.CallbackFailure(envelope);
        failure.initCause(cause);
        return failure;
    }

    static long handle(JsonNode value) {
        try {
            HandleId id = value.isObject() ? RecorderHandleResult.fromJson(value).getHandle() : HandleId.fromJson(value);
            if ("0".equals(id.getValue())) throw new IllegalStateException("Expected a nonzero handle");
            return WireJson.u64(id.getValue()).longValue();
        } catch (IllegalArgumentException error) {
            throw new IllegalStateException("Invalid native handle", error);
        }
    }
}
