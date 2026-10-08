package ai.arcships.aimux;

import com.sun.jna.Pointer;
import com.sun.jna.ptr.LongByReference;
import com.sun.jna.ptr.PointerByReference;
import java.io.Closeable;
import java.util.Objects;
import java.util.concurrent.atomic.AtomicLong;

/** Native decisions. Requests, results and capabilities use the core JSON contract. */
public final class DecisionModel implements Closeable {
    private final AtomicLong handle;

    DecisionModel(long handle) { this.handle = new AtomicLong(handle); }

    public static DecisionModel jev(String apiKey, String modelId) {
        return jev(apiKey, modelId, null, null);
    }

    /** Endpoint is a complete POST URL; null uses the official API. */
    public static DecisionModel jev(String apiKey, String modelId, String endpoint, String probabilitySource) {
        Objects.requireNonNull(apiKey, "apiKey");
        Objects.requireNonNull(modelId, "modelId");
        LongByReference out = new LongByReference();
        Pointer error = AimuxFFI.INSTANCE.aimux_jev_decision_new_with_probability_source(
            apiKey, modelId, endpoint, probabilitySource, out);
        return new DecisionModel(AimuxResult.extractHandle(error, out, "jev decision"));
    }

    public String decide(String optsJson) { return decide(optsJson, 0L); }

    /** Optional abort handle uses the existing C ABI abort signal lifecycle. */
    public String decide(String optsJson, long abortHandle) {
        AimuxResult.requireJsonNonNull(optsJson, "optsJson");
        PointerByReference out = new PointerByReference();
        return AimuxResult.extractString(
            AimuxFFI.INSTANCE.aimux_decide_with_abort(requireHandle(), optsJson, abortHandle, out), out, "decide");
    }

    /** Query capabilities and rounding precision without HTTP. */
    public String capabilities() {
        PointerByReference out = new PointerByReference();
        return AimuxResult.extractString(
            AimuxFFI.INSTANCE.aimux_decision_capabilities(requireHandle(), out), out, "decision capabilities");
    }

    private long requireHandle() {
        long value = handle.get();
        if (value == 0L) throw new IllegalStateException("DecisionModel is closed");
        return value;
    }

    @Override public void close() {
        long value = handle.getAndSet(0L);
        if (value != 0L) AimuxFFI.INSTANCE.aimux_drop_handle(value);
    }

    @Override protected void finalize() throws Throwable {
        close();
        super.finalize();
    }
}
