package io.aimux;

import com.fasterxml.jackson.databind.JsonNode;

/**
 * Package-private helpers for FFI string extraction and error-envelope
 * checking, shared by the multimodal model classes.
 *
 * <p>Mirrors the Kotlin binding's {@code extractFFIString} /
 * {@code throwIfErrorEnvelope} helpers (Multimodal.kt): every raw multimodal
 * method frees the native allocation, and surfaces a {@code {"error":"..."}}
 * envelope as an {@link AimuxException} rather than returning it silently.
 */
final class AimuxResult {
    private AimuxResult() {}

    /**
     * Read a caller-owned UTF-8 string from an FFI return pointer, free it, and
     * throw {@link AimuxException} if it is an error envelope.
     *
     * @param ptr     the pointer returned by an {@code aimux_*} function (may be null)
     * @param context method name for the null-pointer error message
     * @return the result string (never an error envelope)
     */
    static String extractString(com.sun.jna.Pointer ptr, String context) {
        if (ptr == null) {
            throw new RuntimeException(context + " returned null");
        }
        try {
            String result = ptr.getString(0, "UTF-8");
            throwIfErrorEnvelope(result);
            return result;
        } finally {
            AimuxFFI.INSTANCE.aimux_free_string(ptr);
        }
    }

    /**
     * If {@code result} is a {@code {"error":"..."}} envelope, throw
     * {@link AimuxException} with the message. Otherwise return.
     */
    static void throwIfErrorEnvelope(String result) {
        String trimmed = result.trim();
        if (trimmed.isEmpty() || trimmed.charAt(0) != '{') {
            return;
        }
        try {
            JsonNode node = Types.AimuxJson.MAPPER.readTree(result);
            JsonNode err = node.get("error");
            if (err != null && err.isTextual()) {
                throw new AimuxException(err.asText());
            }
        } catch (AimuxException e) {
            throw e;
        } catch (Exception ignored) {
            // Not valid JSON or no error field — not an error envelope.
        }
    }
}
