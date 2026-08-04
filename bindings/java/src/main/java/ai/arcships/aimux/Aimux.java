package ai.arcships.aimux;

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
}
