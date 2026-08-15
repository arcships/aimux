package ai.arcships.aimux

import org.assertj.core.api.Assertions.assertThat
import org.junit.jupiter.api.AfterEach
import org.junit.jupiter.api.BeforeEach
import org.junit.jupiter.api.Test
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket

// ─────────────────────────────────────────────────────────────────────────────
// TranscriptionSession nextPart tests (issue #116).
//
// The realtime transcription session streams over a WebSocket
// (`ws://{base}/realtime?intent=transcription`). We point an OpenAI realtime
// transcription model at a **stalling** TCP server — it accepts connections
// but never completes the WebSocket handshake — so the session's connect
// hangs and no part ever arrives. nextPart(timeoutMs) then hits its wait
// bound and the FFI returns NULL + AIMUX_E_TIMEOUT, which the binding must
// surface as the catchable retryable sentinel
// TranscriptionSession.AimuxTranscriptionTimeoutException — not as the
// generic TimeoutError (AimuxException), which a sentinel-catch could never
// intercept.
//
// Requires the native library (same as the other E2E tests); no real network
// access — everything stays on 127.0.0.1.
// ─────────────────────────────────────────────────────────────────────────────

class TranscriptionSessionTest {

    /** Accepted-but-stalled sockets; held open so the WS handshake hangs. */
    private val accepted = java.util.concurrent.CopyOnWriteArrayList<Socket>()

    private lateinit var server: ServerSocket
    private lateinit var acceptThread: Thread

    @BeforeEach
    fun setUp() {
        server = ServerSocket(0, 8, InetAddress.getByName("127.0.0.1"))
        acceptThread = Thread {
            while (!server.isClosed) {
                try {
                    accepted.add(server.accept())
                } catch (_: java.io.IOException) {
                    return@Thread
                }
            }
        }
        acceptThread.isDaemon = true
        acceptThread.start()
    }

    @AfterEach
    fun tearDown() {
        runCatching { server.close() }
        accepted.forEach { runCatching { it.close() } }
    }

    private val baseUrl: String
        get() = "http://127.0.0.1:${server.localPort}"

    @Test
    fun `nextPart timeout throws the retryable AimuxTranscriptionTimeoutException and the session stays live`() {
        TranscriptionModel.openai("sk-test", "gpt-realtime-whisper", baseUrl).use { model ->
            model.startStream().use { session ->
                // The WS handshake to the stalling server never completes, so
                // no part arrives within the wait bound: the documented
                // sentinel must be thrown (previously dead code — the
                // throwFromC(TimeoutError) before it never returned).
                val ex = catchSentinel { session.nextPart(150) }
                assertThat(ex).isNotNull
                assertThat(ex).isInstanceOf(RuntimeException::class.java)

                // It must NOT be the generic engine TimeoutError: callers
                // catching the sentinel per the KDoc need it to match, and a
                // TimeoutError catch-all is not how "session still live,
                // call again" is expressed (RFC-0028 AIMUX_E_TIMEOUT).
                assertThat(ex).isNotInstanceOf(TimeoutError::class.java)
                assertThat(ex).isNotInstanceOf(AimuxException::class.java)

                // Retryable: after a timeout the session is still alive — a
                // second pull again yields the sentinel, not a stream-failure
                // or invalid-handle error.
                val second = catchSentinel { session.nextPart(150) }
                assertThat(second).isNotNull
            }
        }
    }

    @Test
    fun `nextPart maps non-timeout failures to the typed AimuxException hierarchy`() {
        // `whisper-1` is not a realtime model: do_stream rejects it before
        // connecting (UnsupportedFunctionality), which surfaces on the first
        // nextPart as a typed engine error — never as the timeout sentinel.
        TranscriptionModel.openai("sk-test", "whisper-1", baseUrl).use { model ->
            model.startStream().use { session ->
                val ex = try {
                    session.nextPart(2_000)
                    null
                } catch (e: AimuxException) {
                    e
                }
                assertThat(ex).isInstanceOf(UnsupportedFunctionalityError::class.java)
                assertThat(ex!!.code).isEqualTo(AIMUX_E_UNSUPPORTED_FUNCTIONALITY)
            }
        }
    }

    /** Run [block], returning the timeout sentinel if it was thrown. */
    private inline fun catchSentinel(block: () -> Unit): TranscriptionSession.AimuxTranscriptionTimeoutException? =
        try {
            block()
            null
        } catch (e: TranscriptionSession.AimuxTranscriptionTimeoutException) {
            e
        }
}
