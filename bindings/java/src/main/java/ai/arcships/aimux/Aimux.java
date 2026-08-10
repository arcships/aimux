package ai.arcships.aimux;

import com.sun.jna.Library;
import com.sun.jna.Native;

/**
 * Top-level entry point for global aimux services (RFC-0014 logging).
 *
 * <p>Unlike the model classes (which wrap one native handle each), the
 * logging entry is a process-global, idempotent operation.
 */
public final class Aimux {

    private Aimux() {
        // Utility class — no instances.
    }

    /**
     * Initialize the global logger (RFC-0014).
     *
     * <p>Idempotent — safe to call any number of times from any thread; only
     * the first call has an effect. If the host already registered its own
     * {@code tracing} subscriber, this is a no-op (aimux never overrides a
     * consumer's logger).
     *
     * @param level "off" | "error" | "warn" | "info" | "debug" | "trace";
     *              {@code null} or empty defaults to "warn". The
     *              {@code AIMUX_LOG} and {@code AIMUX_LOG_LEVEL} environment
     *              variables take precedence when set. Logs go to stderr.
     */
    public static void initLogging(String level) {
        String effective = (level == null || level.isEmpty()) ? "warn" : level;
        AimuxFFI.INSTANCE.aimux_init_logging(effective);
    }

    /**
     * Start recording (RFC-0023): complete Recording JSONL is written to
     * {@code {dir}/recordings.jsonl} (the directory is auto-created).
     *
     * <p>Recording is opt-in and process-global; calling this again replaces
     * the previous recorder.
     *
     * @param dir Directory to write {@code recordings.jsonl} into.
     * @throws IllegalArgumentException if {@code dir} is null.
     */
    public static void initRecording(String dir) {
        if (AimuxFFI.INSTANCE.aimux_init_recording(dir) != 0) {
            throw new IllegalArgumentException("Failed to initialize recording: dir must be non-null");
        }
    }

    /**
     * Local JNA binding for the no-arg default-capacity ring entry point.
     * Declared here (rather than the shared {@code AimuxFFI} interface) to keep
     * this change scoped to {@code Aimux.java}. JNA caches the underlying native
     * library by name, so loading {@code aimux_ffi} again for this interface
     * reuses the already-open handle.
     */
    private interface RecordingDefaultFFI extends Library {
        RecordingDefaultFFI INSTANCE = Native.load("aimux_ffi", RecordingDefaultFFI.class);
        int aimux_init_recording_ring_default();
    }

    /**
     * Start in-memory bounded recording with the library default capacity
     * (RingRecorder, FIFO eviction; RFC-0023). Convenience overload — callers
     * who don't need a specific cap should prefer this.
     */
    public static void initRecordingRing() {
        if (RecordingDefaultFFI.INSTANCE.aimux_init_recording_ring_default() != 0) {
            throw new IllegalArgumentException("Failed to initialize default ring recording");
        }
    }

    /**
     * Start in-memory bounded recording (RingRecorder, FIFO eviction; RFC-0023).
     *
     * @param cap Maximum number of in-memory recordings before old ones are evicted.
     * @throws IllegalArgumentException if {@code cap <= 0}. The bound is checked
     *                                  before the FFI call so a negative Java
     *                                  {@code long} is never reinterpreted by
     *                                  JNA/the C ABI as a huge {@code uint64_t}.
     */
    public static void initRecordingRing(long cap) {
        if (cap <= 0) {
            throw new IllegalArgumentException("initRecordingRing: cap must be > 0");
        }
        if (AimuxFFI.INSTANCE.aimux_init_recording_ring(cap) != 0) {
            throw new IllegalArgumentException("Failed to initialize ring recording");
        }
    }

    /**
     * Stop recording (RFC-0023): the global recorder becomes {@code None}.
     * Idempotent — safe to call when no recording is active.
     */
    public static void recordingStop() {
        AimuxFFI.INSTANCE.aimux_recording_stop();
    }

    /**
     * Flush the global recorder (RFC-0023): blocks until the JSONL is on disk
     * (no-op for the ring recorder).
     */
    public static void recordingFlush() {
        AimuxFFI.INSTANCE.aimux_recording_flush();
    }
}
