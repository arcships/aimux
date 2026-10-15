package ai.arcships.aimux;

/** Explicit cancellation, independent of closing model resources. */
public final class Abort extends NativeHandle {
    Abort(Aimux aimux, long id) { super(aimux, id); }
    public void signal() {
        try (Lease lease = admit()) { aimux.ffi.aimux_abort(lease.id); }
    }
}
