package ai.arcships.aimux;

import ai.arcships.aimux.types.ErrorEnvelope;

import com.fasterxml.jackson.databind.JsonNode;

/** Recording domain errors from the RFC-0039 envelope. */
public abstract class RecordingException extends NativeException {
    RecordingException(ErrorEnvelope envelope) { super(envelope); }
    static RecordingException decode(ErrorEnvelope envelope) {
        switch (envelope.getError().path("name").asText()) {
            case "Init": return new Init(envelope);
            case "OpenFile": return new OpenFile(envelope);
            case "Spawn": return new Spawn(envelope);
            case "WriterGone": return new WriterGone(envelope);
            case "FlushTimeout": return new FlushTimeout(envelope);
            case "Write": return new Write(envelope);
            default: return new Unknown(envelope);
        }
    }

    public static final class Init extends RecordingException {
        Init(ErrorEnvelope envelope) { super(envelope); }
    }
    public static final class OpenFile extends RecordingException {
        OpenFile(ErrorEnvelope envelope) { super(envelope); }
    }
    public static final class Spawn extends RecordingException {
        Spawn(ErrorEnvelope envelope) { super(envelope); }
    }
    public static final class WriterGone extends RecordingException {
        WriterGone(ErrorEnvelope envelope) { super(envelope); }
    }
    public static final class FlushTimeout extends RecordingException {
        FlushTimeout(ErrorEnvelope envelope) { super(envelope); }
    }
    public static final class Write extends RecordingException {
        Write(ErrorEnvelope envelope) { super(envelope); }
    }
    /** An unrecognized variant, retaining its complete envelope. */
    public static final class Unknown extends RecordingException {
        Unknown(ErrorEnvelope envelope) { super(envelope); }
    }
}
