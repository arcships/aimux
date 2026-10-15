package ai.arcships.aimux;

/** Owned resource for any manifest handle kind. Only legal op/kind combinations are accepted natively. */
public final class Resource extends NativeHandle {
    Resource(Aimux aimux, long id) { super(aimux, id); }
}
