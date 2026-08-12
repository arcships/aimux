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

    /**
     * Register external OpenAI-compatible providers from a JSON config string
     * (RFC-0020).
     *
     * <p>{@code configJson} is {@code { "providers": [ { "name", "base_url", ... } ] }}.
     * Entries override same-named built-ins or add new ones. Like
     * {@link #initRecording}, this mutates process-global registry state.
     *
     * <p>The C entry point returns an {@code int} (1 = success, 0 = failure)
     * rather than a handle, so the rc is checked inline (no
     * {@code AimuxResult.extractHandle}).
     *
     * @param configJson Provider registry config JSON.
     * @throws AimuxException if the C call fails (rc == 0).
     */
    public static void registerProviders(String configJson) {
        AimuxCError err = AimuxResult.newError();
        int rc = AimuxFFI.INSTANCE.aimux_register_providers(configJson, err);
        if (rc == 0) {
            throw AimuxException.fromC(err, "registerProviders");
        }
    }

    /**
     * Set the global proxy configuration (M6, RFC-0016). Must be called before
     * the first {@code generateText} / {@code streamText} call; a no-op if the
     * shared HTTP client is already initialised.
     *
     * <p>The C entry point returns an {@code int} (1 = success, 0 = failure).
     *
     * @param configJson ProxyConfig JSON ({@code "http_url"}, {@code "https_url"},
     *                   {@code "all_url"}, {@code "no_proxy"} — all optional).
     * @throws AimuxException if the C call fails (rc == 0).
     */
    public static void initProxy(String configJson) {
        AimuxCError err = AimuxResult.newError();
        int rc = AimuxFFI.INSTANCE.aimux_init_proxy(configJson, err);
        if (rc == 0) {
            throw AimuxException.fromC(err, "initProxy");
        }
    }
}
