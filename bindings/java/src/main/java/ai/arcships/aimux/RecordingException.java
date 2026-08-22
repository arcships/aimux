package ai.arcships.aimux;

import com.sun.jna.Pointer;

/**
 * Recording (RFC-0023) failure from {@link Aimux#initRecording(String)} /
 * {@link Aimux#recordingTryFlush()}.
 *
 * <p>Deliberately <em>not</em> an {@link AimuxException}: {@code AiMuxError} and
 * {@code recording::RecordingError} are unrelated Rust types, and this binding
 * mirrors that — the two share only {@link RuntimeException}.
 * Message text comes from the C layer.
 */
public class RecordingException extends RuntimeException {

    private static final long serialVersionUID = 1L;

    private final RecordingErrorCode code;

    public RecordingException(RecordingErrorCode code, String message) {
        super(message);
        this.code = code;
    }

    public RecordingErrorCode getCode() {
        return code;
    }

    /**
     * Build from a returned {@code const aimux_error_t *}. Reads the unified
     * code and message, frees the string; the caller
     * ({@link AimuxResult#expectRecordingError}) frees the returned error.
     * Separate from {@link AimuxException#fromC} on purpose.
     */
    static RecordingException fromC(Pointer error, String prefix) {
        AimuxFFI ffi = AimuxFFI.INSTANCE;
        RecordingErrorCode code = RecordingErrorCode.fromC(ffi.aimux_error_code(error));
        String msg = AimuxResult.takeString(ffi.aimux_error_message(error));
        if (msg == null || msg.isEmpty()) {
            msg = "aimux: recording " + code;
        }
        return new RecordingException(code, prefix + msg);
    }
}
