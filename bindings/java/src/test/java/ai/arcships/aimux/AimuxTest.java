package ai.arcships.aimux;

import org.junit.jupiter.api.Test;

import static org.assertj.core.api.Assertions.assertThatCode;
import static org.assertj.core.api.Assertions.assertThatThrownBy;

/**
 * Tests for the top-level {@link Aimux} entry point.
 *
 * <p>T11: {@link Aimux#initRecordingRing(long)} must reject {@code cap <= 0}
 * before the FFI call so a negative Java {@code long} is never reinterpreted by
 * JNA / the C ABI as a huge {@code uint64_t}. The cases below throw before
 * touching {@code AimuxFFI.INSTANCE}, so they need no native library.
 */
class AimuxTest {

    @Test
    void initRecordingRingRejectsZeroCap() {
        assertThatThrownBy(() -> Aimux.initRecordingRing(0L))
            .isInstanceOf(IllegalArgumentException.class)
            .hasMessage("initRecordingRing: cap must be > 0");
    }

    @Test
    void initRecordingRingRejectsNegativeCap() {
        assertThatThrownBy(() -> Aimux.initRecordingRing(-1L))
            .isInstanceOf(IllegalArgumentException.class)
            .hasMessage("initRecordingRing: cap must be > 0");
    }

    @Test
    void initRecordingRingNoArgUsesDefault() {
        // The no-arg overload uses the library default capacity (FFI
        // aimux_init_recording_ring_default). Unlike the cap-validation tests
        // above, this one reaches the FFI and requires the native library on
        // java.library.path / LD_LIBRARY_PATH.
        assertThatCode(() -> Aimux.initRecordingRing()).doesNotThrowAnyException();
        Aimux.recordingStop();
    }
}
