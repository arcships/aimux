package ai.arcships.aimux

import com.sun.jna.Library
import com.sun.jna.Native

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
 * @throws IllegalArgumentException if `dir` is null/empty (the C ABI returns -1).
 */
fun initRecording(dir: String) {
    if (dir.isEmpty()) throw IllegalArgumentException("initRecording: dir must not be empty")
    requireOk(FFI.lib.aimux_init_recording(dir), "initRecording")
}

/**
 * Local JNA binding for the no-arg default-capacity ring entry point. Declared
 * here (rather than the shared `AimuxFFI` interface in Model.kt) to keep this
 * change scoped to Recording.kt. JNA caches the underlying native library by
 * name, so loading "aimux_ffi" again for this interface reuses the already-open
 * handle.
 */
private interface RecordingDefaultFFI : Library {
    fun aimux_init_recording_ring_default(): Int
}

private val recordingDefaultLib: RecordingDefaultFFI =
    Native.load("aimux_ffi", RecordingDefaultFFI::class.java)

/**
 * Start in-memory bounded recording (`RingRecorder`, FIFO eviction).
 *
 * @param cap Maximum number of entries held in memory; `null` (default) uses
 *   the library default capacity (FFI `aimux_init_recording_ring_default`).
 * @throws IllegalArgumentException if `cap <= 0` (the C ABI returns -1).
 */
fun initRecordingRing(cap: Int? = null) {
    if (cap == null) {
        requireOk(recordingDefaultLib.aimux_init_recording_ring_default(), "initRecordingRing")
        return
    }
    if (cap <= 0) throw IllegalArgumentException("initRecordingRing: cap must be > 0")
    requireOk(FFI.lib.aimux_init_recording_ring(cap.toLong()), "initRecordingRing")
}

/** Stop recording: the global recorder becomes None (new calls are unrecorded). */
fun recordingStop() {
    requireOk(FFI.lib.aimux_recording_stop(), "recordingStop")
}

/**
 * Flush the global recorder (blocks until the JSONL is on disk; no-op for the
 * ring recorder).
 */
fun recordingFlush() {
    requireOk(FFI.lib.aimux_recording_flush(), "recordingFlush")
}

private fun requireOk(code: Int, context: String) {
    if (code != 0) throw IllegalArgumentException("$context: native call failed (code $code)")
}
