package ai.arcships.aimux;

import ai.arcships.aimux.generated.AimuxOps;

import com.fasterxml.jackson.databind.JsonNode;
import com.sun.jna.ptr.IntByReference;
import com.sun.jna.ptr.PointerByReference;
import java.util.Objects;

/** An Operation(transcription); poll timeout is distinct from operation failure. */
public final class TranscriptionSession extends NativeHandle {
    TranscriptionSession(Aimux aimux, long id) { super(aimux, id); }

    public void push(byte[] bytes) {
        Objects.requireNonNull(bytes, "bytes");
        try (Lease lease = admit()) {
            AimuxResult.check(aimux.ffi,
                aimux.ffi.aimux_operation_push(lease.id, bytes, new AimuxFFI.SizeT(bytes.length)));
        }
    }

    public void inputDone() { call(AimuxOps.OP_OPERATION_INPUT_DONE, "{}"); }
    public JsonNode status() { return call(AimuxOps.OP_OPERATION_STATUS, "{}"); }
    public JsonNode cancel() { return call(AimuxOps.OP_OPERATION_CANCEL, "{}"); }

    public PollResult next(long timeoutMs) {
        PointerByReference part = new PointerByReference();
        IntByReference state = new IntByReference();
        try (Lease lease = admit()) {
            try {
                AimuxResult.check(aimux.ffi,
                    aimux.ffi.aimux_operation_next_part(lease.id, timeoutMs, part, state));
                switch (state.getValue()) {
                    case 1:
                        com.sun.jna.Pointer pointer = part.getValue();
                        part.setValue(null);
                        String json = AimuxResult.takeString(aimux.ffi, pointer);
                        if (json == null) throw new IllegalStateException("PART has no JSON");
                        return new PollResult(PollResult.State.PART, AimuxResult.parse(json));
                    case 2: return new PollResult(PollResult.State.ENDED, null);
                    case 3: return new PollResult(PollResult.State.TIMEOUT, null);
                    default: throw new IllegalStateException("Unknown native poll state");
                }
            } finally {
                if (part.getValue() != null) aimux.ffi.aimux_free_string(part.getValue());
            }
        }
    }

    public static final class PollResult {
        public enum State { PART, ENDED, TIMEOUT }
        private final State state;
        private final JsonNode part;
        private PollResult(State state, JsonNode part) { this.state = state; this.part = part; }
        public State getState() { return state; }
        public JsonNode getPart() { return part == null ? null : part.deepCopy(); }
    }
}
