package ai.arcships.aimux;

import org.junit.jupiter.api.Test;

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
}
