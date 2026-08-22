package ai.arcships.aimux;

import com.sun.jna.Pointer;
import com.sun.jna.ptr.IntByReference;
import com.sun.jna.ptr.PointerByReference;

/**
 * A live streaming-transcription session (RFC-0028): push audio chunks with
 * {@link #pushAudio(byte[])}, mark end-of-audio with {@link #inputDone()},
 * then pull transcription parts (JSON {@code TranscriptionStreamPart}) with
 * {@link #nextPart(long)}.
 *
 * <p>Close releases the session (aborts the driver; idempotent). Use
 * try-with-resources — {@link #close()} is the primary release path.
 *
 * <p>Sessions are created via {@link TranscriptionModel#startStream(String)}.
 */
public final class TranscriptionSession implements AutoCloseable {

    private final java.util.concurrent.atomic.AtomicLong handle;

    TranscriptionSession(long handle) {
        this.handle = new java.util.concurrent.atomic.AtomicLong(handle);
    }

    /** Release the native session. Idempotent and thread-safe. */
    @Override
    public void close() {
        long h = handle.getAndSet(0L);
        if (h != 0L) AimuxFFI.INSTANCE.aimux_transcription_session_drop(h);
    }

    @Override
    protected void finalize() throws Throwable {
        close();
        super.finalize();
    }

    private long requireHandle() {
        long h = handle.get();
        if (h == 0L) throw new IllegalStateException("TranscriptionSession is closed");
        return h;
    }

    /**
     * Push one binary audio chunk. <b>Blocks</b> while the internal channel is
     * full (backpressure propagation — the capture loop throttles).
     *
     * @param audio audio bytes (may be empty).
     * @throws AimuxException on failure (session ended / input finished).
     */
    public void pushAudio(byte[] audio) {
        long h = requireHandle();
        byte[] data = audio == null ? new byte[0] : audio;
        Pointer e = AimuxFFI.INSTANCE.aimux_transcription_push_audio(h, data, data.length);
        if (e != null) throw AimuxResult.expectAimuxError(e, "pushAudio failed");
    }

    /**
     * Signal end-of-audio (idempotent).
     *
     * @throws IllegalStateException on a dead native handle (C ABI failure).
     */
    public void inputDone() {
        long h = requireHandle();
        Pointer e = AimuxFFI.INSTANCE.aimux_transcription_input_done(h);
        if (e != null) throw AimuxResult.expectFfiError(e, "inputDone failed");
    }

    /**
     * Pull the next transcription part (JSON {@code TranscriptionStreamPart}).
     *
     * @param timeoutMs wait bound: {@code >0} wait at most; {@code 0} immediate
     *                  poll; {@code <0} wait indefinitely.
     * @return the part JSON.
     * @throws AimuxTranscriptionEndedException   the stream finished normally
     *                                            (a Finish part was delivered).
     * @throws AimuxTranscriptionTimeoutException no part arrived in time; the
     *                                            session stays live — call again.
     * @throws AimuxException                     the stream failed.
     */
    public String nextPart(long timeoutMs) {
        long h = requireHandle();
        PointerByReference outPart = new PointerByReference();
        IntByReference outState = new IntByReference();
        Pointer e = AimuxFFI.INSTANCE.aimux_transcription_next_part(h, timeoutMs, outPart, outState);
        if (e != null) {
            throw AimuxResult.expectAimuxError(e, "nextPart failed");
        }
        switch (outState.getValue()) {
            case AimuxFFI.TRANSCRIPTION_NEXT_PART_PART:
                return AimuxResult.extractString(null, outPart, "nextPart");
            case AimuxFFI.TRANSCRIPTION_NEXT_PART_ENDED:
                throw new AimuxTranscriptionEndedException();
            case AimuxFFI.TRANSCRIPTION_NEXT_PART_TIMEOUT:
                // Not an error: nothing to decode, the session stays live.
                throw new AimuxTranscriptionTimeoutException();
            default:
                throw new IllegalStateException(
                    "nextPart: aimux ffi: unknown next_part state " + outState.getValue());
        }
    }

    /** The transcription stream ended normally. */
    public static class AimuxTranscriptionEndedException extends RuntimeException {
        public AimuxTranscriptionEndedException() {
            super("transcription stream ended");
        }
    }

    /** No transcription part arrived within the timeout (retryable). */
    public static class AimuxTranscriptionTimeoutException extends RuntimeException {
        public AimuxTranscriptionTimeoutException() {
            super("transcription part timeout");
        }
    }
}
