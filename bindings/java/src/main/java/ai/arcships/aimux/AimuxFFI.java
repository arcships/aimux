package ai.arcships.aimux;

import com.sun.jna.Callback;
import com.sun.jna.IntegerType;
import com.sun.jna.Library;
import com.sun.jna.Native;
import com.sun.jna.Pointer;
import com.sun.jna.ptr.IntByReference;
import com.sun.jna.ptr.PointerByReference;

/** The versioned C ABI. All strings are UTF-8; returned strings are owned. */
public interface AimuxFFI extends Library {
    /** Native size_t is pointer-sized on both LLP64 and LP64. */
    final class SizeT extends IntegerType {
        public SizeT() { this(0); }
        public SizeT(long value) {
            super(Native.SIZE_T_SIZE, value, true);
            if (value < 0 || (Native.SIZE_T_SIZE == 4 && value > 0xffffffffL)) {
                throw new IllegalArgumentException("size_t out of range");
            }
        }
    }

    interface EventCallback extends Callback {
        void invoke(Pointer frameJson, Pointer context);
    }

    int aimux_abi_version();
    Pointer aimux_manifest_json(PointerByReference outJson);
    Pointer aimux_call(long handle, String op, String paramsJson, long abort, PointerByReference outJson);
    Pointer aimux_stream(long handle, String op, String paramsJson, long abort,
                         EventCallback onEvent, Pointer context);
    Pointer aimux_operation_push(long operation, byte[] data, SizeT length);
    Pointer aimux_operation_next_part(long operation, long timeoutMs,
                                      PointerByReference outPart, IntByReference outState);
    void aimux_abort(long abort);
    void aimux_drop(long handle);
    void aimux_handle_finalizer(Pointer token);
    void aimux_free_string(Pointer string);
    void aimux_error_free(Pointer error);
    int aimux_error_code(Pointer error);
    Pointer aimux_error_message(Pointer error);
    Pointer aimux_error_json(Pointer error);
}
