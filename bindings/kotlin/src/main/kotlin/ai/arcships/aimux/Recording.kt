package ai.arcships.aimux

/**
 * Recording + mock replay (RFC-0023).
 *
 * Recording is **opt-in** and global: nothing is recorded until
 * [initRecording] or [initRecordingRing] is called. Calling one of the init
 * functions again replaces the global recorder.
 */

/**
 * Start recording: the complete `Recording` JSONL is written to
 * `{dir}/recordings.jsonl` (dir is auto-created).
 *
 * @throws IllegalArgumentException if `dir` is empty.
 * @throws RecordingException (code `INIT` — dir could not be created,
 *   `OPEN_FILE` or `SPAWN`) if the recorder could not be constructed. On
 *   failure the previous recorder (if any) stays in place.
 */
fun initRecording(dir: String) {
    if (dir.isEmpty()) throw IllegalArgumentException("initRecording: dir must not be empty")
    FFI.lib.aimux_init_recording(dir)?.let { throw expectRecordingError(it, "initRecording") }
}

/**
 * Start in-memory bounded recording (`RingRecorder`, FIFO eviction).
 *
 * @param cap Maximum number of entries held in memory; `null` (default) uses
 *   the library default capacity (FFI `aimux_init_recording_ring_default`).
 * @throws IllegalArgumentException if `cap <= 0` (checked before the C call so a
 *   negative value is never reinterpreted as a huge `uint64_t`).
 */
fun initRecordingRing(cap: Int? = null) {
    if (cap == null) {
        FFI.lib.aimux_init_recording_ring_default()
        return
    }
    if (cap <= 0) throw IllegalArgumentException("initRecordingRing: cap must be > 0")
    FFI.lib.aimux_init_recording_ring(cap.toLong())?.let { throw expectAimuxError(it, "initRecordingRing") }
}

/** Stop recording: the global recorder becomes None (new calls are unrecorded). */
fun recordingStop() {
    FFI.lib.aimux_recording_stop()
}

/**
 * Flush the global recorder (blocks until the JSONL is on disk; no-op for the
 * ring recorder).
 */
fun recordingFlush() {
    FFI.lib.aimux_recording_flush()
}

/**
 * Checked flush: like [recordingFlush] but reports failures. Throws
 * [RecordingException] (code `WRITE`, `WRITER_GONE` or `FLUSH_TIMEOUT`) — not
 * an [AimuxException]. Returns normally when nothing is recording. The legacy
 * [recordingFlush] stays and never reports.
 */
fun recordingTryFlush() {
    FFI.lib.aimux_recording_try_flush()?.let { throw expectRecordingError(it, "recordingTryFlush") }
}
