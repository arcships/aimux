package ai.arcships.aimux;

import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

import static org.assertj.core.api.Assertions.assertThat;
import static org.assertj.core.api.Assertions.assertThatCode;
import static org.assertj.core.api.Assertions.assertThatThrownBy;

/**
 * {@link Aimux#initRecording(String)} / {@link Aimux#recordingTryFlush()}: recording
 * failures are reported as {@link RecordingException} — its own type, not an
 * {@link AimuxException}.
 */
class RecordingTest {

    @AfterEach
    void stopRecording() {
        Aimux.recordingStop();
    }

    @Test
    void tryFlushSucceedsWhenNothingIsRecording() {
        Aimux.recordingStop();
        assertThatCode(Aimux::recordingTryFlush).doesNotThrowAnyException();
    }

    @Test
    void initRecordingReportsInitForUnwritableDir(@TempDir Path tmp) throws IOException {
        // Parent path is a regular file: the recorder cannot create its
        // directory → init fails with INIT and nothing is installed.
        Path blocker = tmp.resolve("blocker");
        Files.write(blocker, new byte[] {'x'});
        Aimux.recordingStop();

        assertThatThrownBy(() -> Aimux.initRecording(blocker.resolve("rec").toString()))
            .isInstanceOf(RecordingException.class)
            .isNotInstanceOf(AimuxException.class)
            .satisfies(e -> assertThat(((RecordingException) e).getCode())
                .isEqualTo(RecordingErrorCode.INIT))
            .hasMessageStartingWith("initRecording: ");

        // Nothing recording after the failed init → checked flush succeeds.
        assertThatCode(Aimux::recordingTryFlush).doesNotThrowAnyException();
    }
}
