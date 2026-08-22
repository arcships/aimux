package ai.arcships.aimux

/**
 * Initialize the global logger (RFC-0014).
 *
 * Idempotent — safe to call any number of times from any thread; only the
 * first call has an effect. If the host already registered its own `tracing`
 * subscriber, this is a no-op (aimux never overrides a consumer's logger).
 *
 * @param level "off" | "error" | "warn" | "info" | "debug" | "trace";
 *              empty defaults to "warn". The `AIMUX_LOG` and `AIMUX_LOG_LEVEL`
 *              environment variables take precedence when set. Logs go to
 *              stderr.
 */
fun initLogging(level: String = "warn") {
    val effective = level.ifEmpty { "warn" }
    FFI.lib.aimux_init_logging(effective)?.let { throw expectFfiError(it, "initLogging") }
}
