package ai.arcships.aimux;

import com.fasterxml.jackson.databind.JsonNode;
import java.math.BigInteger;

/** One copied stream frame. An accepted stream has exactly one End frame. */
public final class StreamFrame {
    public enum Outcome { SUCCESS, ERROR, CANCELLED }
    private final JsonNode json;
    private final BigInteger sequence;
    private final Outcome outcome;

    private StreamFrame(JsonNode json, BigInteger sequence, Outcome outcome) {
        this.json = json.deepCopy();
        this.sequence = sequence;
        this.outcome = outcome;
    }

    static StreamFrame parse(String value) {
        JsonNode json = AimuxResult.parse(value);
        String seq = json.path("seq").asText();
        if (!json.path("seq").isTextual() || !seq.matches("0|[1-9][0-9]*")) {
            throw new IllegalStateException("Invalid stream sequence");
        }
        BigInteger sequence = new BigInteger(seq);
        if (sequence.bitLength() > 64) throw new IllegalStateException("Stream sequence exceeds uint64");
        String type = json.path("type").asText();
        if ("event".equals(type) && json.has("event")) return new StreamFrame(json, sequence, null);
        if ("end".equals(type)) {
            Outcome outcome;
            switch (json.path("outcome").asText()) {
                case "success": outcome = Outcome.SUCCESS; break;
                case "error": outcome = Outcome.ERROR; break;
                case "cancelled": outcome = Outcome.CANCELLED; break;
                default: throw new IllegalStateException("Invalid stream outcome");
            }
            if (outcome == Outcome.ERROR && !json.hasNonNull("error")) {
                throw new IllegalStateException("Error End is missing its envelope");
            }
            return new StreamFrame(json, sequence, outcome);
        }
        throw new IllegalStateException("Invalid stream frame");
    }

    public BigInteger getSequence() { return sequence; }
    public boolean isEnd() { return outcome != null; }
    public Outcome getOutcome() { return outcome; }
    public JsonNode getEvent() { return json.path("event").deepCopy(); }
    public JsonNode getJson() { return json.deepCopy(); }
    public NativeException getError() {
        return json.hasNonNull("error") ? AimuxResult.decode(json.get("error")) : null;
    }
}
