package ai.arcships.aimux;

/**
 * Raised when the engine returns an {@code {"error": "..."}} result or a
 * stream-level error.
 *
 * <p>The raw {@link Model} layer communicates failures as an error envelope
 * rather than throwing; {@link TypedModel} surfaces those failures as this
 * unchecked exception while keeping the typed happy path clean. Mirrors the
 * Kotlin binding's {@code aimux.AimuxException} (RFC-0013 §4.2).
 */
public class AimuxException extends RuntimeException {

    public AimuxException(String message) {
        super(message);
    }

    public AimuxException(String message, Throwable cause) {
        super(message, cause);
    }
}
