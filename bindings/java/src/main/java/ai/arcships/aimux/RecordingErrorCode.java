package ai.arcships.aimux;

/**
 * Recording (RFC-0023) failure codes. Java keeps the six variants independent
 * of their C transport values 100–105. Only {@link #WRITER_GONE}, {@link #FLUSH_TIMEOUT}
 * and {@link #WRITE} are reachable from {@link Aimux#recordingTryFlush()}.
 */
public enum RecordingErrorCode {
    /** {@code create_dir_all} failed. */
    INIT,
    /** Opening {@code recordings.jsonl} failed. */
    OPEN_FILE,
    /** Writer thread could not be spawned. */
    SPAWN,
    /** Writer thread unavailable. */
    WRITER_GONE,
    /** No writer ack within 30s. */
    FLUSH_TIMEOUT,
    /** A prior write failed (sticky). */
    WRITE;

    /**
     * Map a C {@code aimux_error_code_t} (100–105). Any other value is an ABI
     * contract violation, not a recording-error variant.
     */
    static RecordingErrorCode fromC(int code) {
        if (!isCCode(code)) {
            throw new IllegalStateException("Unknown aimux_error_code_t: " + code);
        }
        return values()[code - 100];
    }

    static boolean isCCode(int code) {
        return code >= 100 && code < 100 + values().length;
    }
}
