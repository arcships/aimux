package ai.arcships.aimux;

import com.sun.jna.Library;
import com.sun.jna.Native;
import com.sun.jna.Pointer;

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
        Pointer e = AimuxFFI.INSTANCE.aimux_init_logging(effective);
        if (e != null) {
            throw AimuxResult.expectFfiError(e, "initLogging");
        }
    }

    /**
     * Start recording (RFC-0023): complete Recording JSONL is written to
     * {@code {dir}/recordings.jsonl} (the directory is auto-created).
     *
     * <p>Recording is opt-in and process-global; calling this again replaces
     * the previous recorder. On failure the previous recorder (if any) stays
     * in place.
     *
     * @param dir Directory to write {@code recordings.jsonl} into.
     * @throws NullPointerException if {@code dir} is null
     * @throws RecordingException with {@link RecordingErrorCode#INIT} (directory
     *         could not be created),
     *         {@link RecordingErrorCode#OPEN_FILE} or {@link RecordingErrorCode#SPAWN}.
     */
    public static void initRecording(String dir) {
        java.util.Objects.requireNonNull(dir, "dir");
        Pointer e = AimuxFFI.INSTANCE.aimux_init_recording(dir);
        if (e != null) {
            throw AimuxResult.expectRecordingError(e, "initRecording");
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
        void aimux_init_recording_ring_default();
    }

    /**
     * Start in-memory bounded recording with the library default capacity
     * (RingRecorder, FIFO eviction; RFC-0023). Convenience overload — callers
     * who don't need a specific cap should prefer this.
     */
    public static void initRecordingRing() {
        RecordingDefaultFFI.INSTANCE.aimux_init_recording_ring_default();
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
        Pointer e = AimuxFFI.INSTANCE.aimux_init_recording_ring(cap);
        if (e != null) {
            throw AimuxResult.expectAimuxError(e, "initRecordingRing");
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
     * Checked flush: like {@link #recordingFlush()} but throws
     * {@link RecordingException} ({@link RecordingErrorCode#WRITE} /
     * {@link RecordingErrorCode#WRITER_GONE} / {@link RecordingErrorCode#FLUSH_TIMEOUT})
     * when the JSONL could not be written. Not an {@link AimuxException}: recording
     * errors are their own type. Returns normally when nothing is recording. The
     * legacy {@link #recordingFlush()} stays and never reports.
     */
    public static void recordingTryFlush() {
        Pointer e = AimuxFFI.INSTANCE.aimux_recording_try_flush();
        if (e != null) {
            throw AimuxResult.expectRecordingError(e, "recordingTryFlush");
        }
    }

    /**
     * Register external OpenAI-compatible providers from a JSON config string
     * (RFC-0020).
     *
     * <p>{@code configJson} is {@code { "providers": [ { "name", "base_url", ... } ] }}.
     * Entries override same-named built-ins or add new ones. Like
     * {@link #initRecording}, this mutates process-global registry state.
     *
     * @param configJson Provider registry config JSON.
     * @throws AimuxException if the registry rejects the config
     *         ({@link AimuxException.InvalidArgumentError}).
     */
    public static void registerProviders(String configJson) {
        AimuxResult.requireJsonNonNull(configJson, "configJson");
        Pointer e = AimuxFFI.INSTANCE.aimux_register_providers(configJson);
        if (e != null) {
            throw AimuxResult.expectAimuxError(e, "registerProviders");
        }
    }

    /**
     * Set the global proxy configuration (M6, RFC-0016). Must be called before
     * the first {@code generateText} / {@code streamText} call; a no-op if the
     * shared HTTP client is already initialised.
     *
     * @param configJson ProxyConfig JSON ({@code "http_url"}, {@code "https_url"},
     *                   {@code "all_url"}, {@code "no_proxy"} — all optional).
     * @throws AimuxException if the config has the wrong shape
     *         ({@link AimuxException.InvalidArgumentError}).
     */
    public static void initProxy(String configJson) {
        AimuxResult.requireJsonNonNull(configJson, "configJson");
        Pointer e = AimuxFFI.INSTANCE.aimux_init_proxy(configJson);
        if (e != null) {
            throw AimuxResult.expectAimuxError(e, "initProxy");
        }
    }
}
