package ai.arcships.aimux;

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
        AimuxCError err = AimuxResult.newError();
        int rc = AimuxFFI.INSTANCE.aimux_transcription_push_audio(h, data, data.length, err);
        if (rc == 0) throw AimuxException.fromC(err, "pushAudio failed");
    }

    /**
     * Signal end-of-audio (idempotent).
     *
     * @throws AimuxException on invalid handle.
     */
    public void inputDone() {
        long h = requireHandle();
        AimuxCError err = AimuxResult.newError();
        int rc = AimuxFFI.INSTANCE.aimux_transcription_input_done(h, err);
        if (rc == 0) throw AimuxException.fromC(err, "inputDone failed");
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
        AimuxCError err = AimuxResult.newError();
        com.sun.jna.Pointer ptr =
                AimuxFFI.INSTANCE.aimux_transcription_next_part(h, timeoutMs, err);
        if (ptr != null) {
            return AimuxResult.extractString(ptr, err, "nextPart");
        }
        // NULL: timeout / ended / error — disambiguate via err.code.
        if (err.code == AimuxException.AIMUX_E_TIMEOUT) {
            // fromC consumes (frees) the message strings, then we swap in the
            // retryable sentinel.
            AimuxException.fromC(err, "nextPart timeout");
            throw new AimuxTranscriptionTimeoutException();
        }
        if (err.code == AimuxException.AIMUX_OK) {
            throw new AimuxTranscriptionEndedException();
        }
        throw AimuxException.fromC(err, "nextPart failed");
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
