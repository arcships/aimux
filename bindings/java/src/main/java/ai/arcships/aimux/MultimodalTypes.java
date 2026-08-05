package ai.arcships.aimux;

import com.fasterxml.jackson.annotation.JsonAutoDetect;
import com.fasterxml.jackson.annotation.JsonCreator;
import com.fasterxml.jackson.annotation.JsonInclude;
import com.fasterxml.jackson.annotation.JsonProperty;
import com.fasterxml.jackson.annotation.PropertyAccessor;
import com.fasterxml.jackson.core.JsonGenerator;
import com.fasterxml.jackson.core.JsonParser;
import com.fasterxml.jackson.databind.DeserializationContext;
import com.fasterxml.jackson.databind.DeserializationFeature;
import com.fasterxml.jackson.databind.JsonDeserializer;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.JsonSerializer;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.SerializerProvider;
import com.fasterxml.jackson.databind.module.SimpleModule;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;

import java.io.IOException;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;

/**
 * aimux — typed multimodal data structures mirroring the aimux-core wire format.
 *
 * <p>Port of the Kotlin binding's {@code MultimodalTypes.kt}. Same shapes as the
 * ts-rs generated {@code .ts} types in {@code bindings/node/src/types/}. Field
 * names are camelCase in Java and map to the wire format's snake_case via
 * {@link JsonProperty}.
 *
 * <p>These types are intentionally lenient on decode (unknown keys ignored,
 * every field has a default) so future engine additions don't break existing
 * clients. The serialization config lives in {@link AimuxJson}. Decode a JSON
 * string returned by a multimodal model with, for example:
 *
 * <pre>{@code
 * MultimodalTypes.EmbeddingResult result = MultimodalTypes.AimuxJson.MAPPER
 *     .readValue(jsonStr, MultimodalTypes.EmbeddingResult.class);
 * }</pre>
 *
 * <p>The {@code Base64}/{@code Binary}/{@code Url} unions ({@link AudioData},
 * {@link ImageOutputs}, {@link VideoData}) are serde-style externally-tagged
 * enums on the wire ({@code {"Base64": ...}} / {@code {"Binary": ...}} /
 * {@code {"Url": ...}}), so each has a custom serializer — the same pattern as
 * {@link Types.FileBytes} in {@code Types.java} and
 * {@code StreamPartSerializer} in the Kotlin binding.
 */
public final class MultimodalTypes {
    private MultimodalTypes() {}

    // ─────────────────────────────────────────────────────────────────────────────
    // Shared ObjectMapper (same config as Types.AimuxJson, plus the multimodal
    // union serializers).
    // ─────────────────────────────────────────────────────────────────────────────

    /** Shared JSON mapper for the multimodal typed layer. */
    public static final class AimuxJson {
        private AimuxJson() {}

        public static final ObjectMapper MAPPER = createMapper();

        private static ObjectMapper createMapper() {
            ObjectMapper m = new ObjectMapper();
            m.configure(DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES, false);
            m.setSerializationInclusion(JsonInclude.Include.NON_NULL);
            m.setVisibility(PropertyAccessor.FIELD, JsonAutoDetect.Visibility.ANY);
            m.setVisibility(PropertyAccessor.GETTER, JsonAutoDetect.Visibility.NONE);
            m.setVisibility(PropertyAccessor.SETTER, JsonAutoDetect.Visibility.NONE);
            m.setVisibility(PropertyAccessor.CREATOR, JsonAutoDetect.Visibility.ANY);

            SimpleModule module = new SimpleModule("aimux-multimodal");
            module.addSerializer(AudioData.class, new AudioDataSerializer());
            module.addDeserializer(AudioData.class, new AudioDataDeserializer());
            module.addSerializer(ImageOutputs.class, new ImageOutputsSerializer());
            module.addDeserializer(ImageOutputs.class, new ImageOutputsDeserializer());
            module.addSerializer(VideoData.class, new VideoDataSerializer());
            module.addDeserializer(VideoData.class, new VideoDataDeserializer());
            module.addSerializer(VideoFileData.class, new VideoFileDataSerializer());
            module.addDeserializer(VideoFileData.class, new VideoFileDataDeserializer());
            module.addSerializer(VideoFile.class, new VideoFileSerializer());
            module.addDeserializer(VideoFile.class, new VideoFileDeserializer());
            m.registerModule(module);
            return m;
        }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Embedding
    // ─────────────────────────────────────────────────────────────────────────────

    /** Token usage for an embedding call (input tokens only). */
    public static class EmbeddingUsage {
        @JsonProperty("tokens") private Long tokens;

        @JsonCreator
        EmbeddingUsage() {}

        private EmbeddingUsage(Long tokens) {
            this.tokens = tokens;
        }

        public static EmbeddingUsage of(Long tokens) {
            return new EmbeddingUsage(tokens);
        }

        public Long getTokens() { return tokens; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private Long tokens;

            public Builder tokens(Long v) { this.tokens = v; return this; }

            public EmbeddingUsage build() { return new EmbeddingUsage(tokens); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof EmbeddingUsage)) return false;
            return Objects.equals(tokens, ((EmbeddingUsage) o).tokens);
        }

        @Override
        public int hashCode() { return Objects.hash(tokens); }

        @Override
        public String toString() { return "EmbeddingUsage(" + tokens + ")"; }
    }

    /** Provider response metadata for embeddings. */
    public static class EmbeddingResponse {
        @JsonProperty("headers") private Map<String, String> headers;
        @JsonProperty("body") private JsonNode body;

        @JsonCreator
        EmbeddingResponse() {}

        private EmbeddingResponse(Map<String, String> headers, JsonNode body) {
            this.headers = headers;
            this.body = body;
        }

        public static EmbeddingResponse of(Map<String, String> headers, JsonNode body) {
            return new EmbeddingResponse(headers, body);
        }

        public Map<String, String> getHeaders() { return headers; }
        public JsonNode getBody() { return body; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private Map<String, String> headers;
            private JsonNode body;

            public Builder headers(Map<String, String> v) { this.headers = v; return this; }
            public Builder body(JsonNode v) { this.body = v; return this; }

            public EmbeddingResponse build() { return new EmbeddingResponse(headers, body); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof EmbeddingResponse)) return false;
            EmbeddingResponse that = (EmbeddingResponse) o;
            return Objects.equals(headers, that.headers)
                && Objects.equals(body, that.body);
        }

        @Override
        public int hashCode() { return Objects.hash(headers, body); }

        @Override
        public String toString() { return "EmbeddingResponse(" + headers + ", " + body + ")"; }
    }

    /** Result of an embedding call. */
    public static class EmbeddingResult {
        @JsonProperty("embeddings") private List<List<Float>> embeddings = new ArrayList<>();
        @JsonProperty("usage") private EmbeddingUsage usage;
        @JsonProperty("provider_metadata") private JsonNode providerMetadata;
        @JsonProperty("response") private EmbeddingResponse response;
        @JsonProperty("warnings") private List<JsonNode> warnings = new ArrayList<>();

        @JsonCreator
        EmbeddingResult() {}

        private EmbeddingResult(List<List<Float>> embeddings, EmbeddingUsage usage,
                                JsonNode providerMetadata, EmbeddingResponse response,
                                List<JsonNode> warnings) {
            this.embeddings = embeddings;
            this.usage = usage;
            this.providerMetadata = providerMetadata;
            this.response = response;
            this.warnings = warnings;
        }

        public List<List<Float>> getEmbeddings() { return embeddings; }
        public EmbeddingUsage getUsage() { return usage; }
        public JsonNode getProviderMetadata() { return providerMetadata; }
        public EmbeddingResponse getResponse() { return response; }
        public List<JsonNode> getWarnings() { return warnings; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private List<List<Float>> embeddings = new ArrayList<>();
            private EmbeddingUsage usage;
            private JsonNode providerMetadata;
            private EmbeddingResponse response;
            private List<JsonNode> warnings = new ArrayList<>();

            public Builder embeddings(List<List<Float>> v) { this.embeddings = v; return this; }
            public Builder usage(EmbeddingUsage v) { this.usage = v; return this; }
            public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }
            public Builder response(EmbeddingResponse v) { this.response = v; return this; }
            public Builder warnings(List<JsonNode> v) { this.warnings = v; return this; }

            public EmbeddingResult build() {
                return new EmbeddingResult(embeddings, usage, providerMetadata, response, warnings);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof EmbeddingResult)) return false;
            EmbeddingResult that = (EmbeddingResult) o;
            return Objects.equals(embeddings, that.embeddings)
                && Objects.equals(usage, that.usage)
                && Objects.equals(providerMetadata, that.providerMetadata)
                && Objects.equals(response, that.response)
                && Objects.equals(warnings, that.warnings);
        }

        @Override
        public int hashCode() { return Objects.hash(embeddings, usage, providerMetadata, response, warnings); }

        @Override
        public String toString() { return "EmbeddingResult(" + embeddings + ")"; }
    }

    /** Options for an embedding call. */
    public static class EmbeddingCallOptions {
        @JsonProperty("values") private List<String> values = new ArrayList<>();
        @JsonProperty("provider_options") private JsonNode providerOptions;
        @JsonProperty("headers") private Map<String, String> headers;

        @JsonCreator
        EmbeddingCallOptions() {}

        private EmbeddingCallOptions(List<String> values, JsonNode providerOptions, Map<String, String> headers) {
            this.values = values;
            this.providerOptions = providerOptions;
            this.headers = headers;
        }

        public static EmbeddingCallOptions of(List<String> values) {
            return new EmbeddingCallOptions(values, null, null);
        }

        public List<String> getValues() { return values; }
        public JsonNode getProviderOptions() { return providerOptions; }
        public Map<String, String> getHeaders() { return headers; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private List<String> values = new ArrayList<>();
            private JsonNode providerOptions;
            private Map<String, String> headers;

            public Builder values(List<String> v) { this.values = v; return this; }
            public Builder providerOptions(JsonNode v) { this.providerOptions = v; return this; }
            public Builder headers(Map<String, String> v) { this.headers = v; return this; }

            public EmbeddingCallOptions build() {
                return new EmbeddingCallOptions(values, providerOptions, headers);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof EmbeddingCallOptions)) return false;
            EmbeddingCallOptions that = (EmbeddingCallOptions) o;
            return Objects.equals(values, that.values)
                && Objects.equals(providerOptions, that.providerOptions)
                && Objects.equals(headers, that.headers);
        }

        @Override
        public int hashCode() { return Objects.hash(values, providerOptions, headers); }

        @Override
        public String toString() { return "EmbeddingCallOptions(" + values + ")"; }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Speech (TTS)
    // ─────────────────────────────────────────────────────────────────────────────

    /**
     * Generated audio: a base64 string or raw binary bytes.
     *
     * <p>Wire format (serde externally-tagged): {@code {"Base64": "..."}} |
     * {@code {"Binary": [n,...]}}.
     */
    public abstract static class AudioData {
        private AudioData() {}

        /** Base64-encoded audio. */
        public static class Base64 extends AudioData {
            private String value = "";

            @JsonCreator
            Base64() {}

            public Base64(String value) { this.value = value; }

            public String getValue() { return value; }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Base64)) return false;
                return Objects.equals(value, ((Base64) o).value);
            }

            @Override
            public int hashCode() { return Objects.hash(value); }

            @Override
            public String toString() { return "AudioData.Base64(" + value + ")"; }
        }

        /** Raw binary audio bytes (each element is a 0–255 byte value). */
        public static class Binary extends AudioData {
            private List<Integer> value = new ArrayList<>();

            @JsonCreator
            Binary() {}

            public Binary(List<Integer> value) { this.value = value; }

            public List<Integer> getValue() { return value; }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Binary)) return false;
                return Objects.equals(value, ((Binary) o).value);
            }

            @Override
            public int hashCode() { return Objects.hash(value); }

            @Override
            public String toString() { return "AudioData.Binary(" + value + ")"; }
        }
    }

    /** Custom (de)serializer for {@link AudioData}: {@code {"Base64": "..."}} | {@code {"Binary": [n,...]}}. */
    public static class AudioDataSerializer extends JsonSerializer<AudioData> {
        @Override
        public void serialize(AudioData value, JsonGenerator gen, SerializerProvider serializers) throws IOException {
            gen.writeStartObject();
            if (value instanceof AudioData.Base64) {
                gen.writeStringField("Base64", ((AudioData.Base64) value).getValue());
            } else if (value instanceof AudioData.Binary) {
                gen.writeArrayFieldStart("Binary");
                for (Integer b : ((AudioData.Binary) value).getValue()) {
                    gen.writeNumber(b);
                }
                gen.writeEndArray();
            } else {
                throw new IOException("Unknown AudioData: " + value);
            }
            gen.writeEndObject();
        }
    }

    /** Custom (de)serializer for {@link AudioData}. */
    public static class AudioDataDeserializer extends JsonDeserializer<AudioData> {
        @Override
        public AudioData deserialize(JsonParser p, DeserializationContext ctxt) throws IOException {
            JsonNode node = p.getCodec().readTree(p);
            if (!node.isObject() || node.size() != 1) {
                throw new IOException("AudioData must be a single-key externally-tagged object, got: " + node);
            }
            String tag = node.fieldNames().next();
            JsonNode inner = node.get(tag);
            switch (tag) {
                case "Base64":
                    return new AudioData.Base64(inner.asText());
                case "Binary": {
                    List<Integer> bytes = new ArrayList<>();
                    if (inner.isArray()) {
                        for (JsonNode item : inner) {
                            bytes.add(item.asInt());
                        }
                    }
                    return new AudioData.Binary(bytes);
                }
                default:
                    throw new IOException("Unknown AudioData tag: '" + tag + "'");
            }
        }
    }

    /** Request metadata for speech generation. */
    public static class SpeechRequest {
        // Core models this as `body: Option<serde_json::Value>` (speech_model.rs)
        // — the raw request body, not a prompt string.
        @JsonProperty("body") private JsonNode body;

        @JsonCreator
        SpeechRequest() {}

        private SpeechRequest(JsonNode body) {
            this.body = body;
        }

        public static SpeechRequest of(JsonNode body) {
            return new SpeechRequest(body);
        }

        public JsonNode getBody() { return body; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private JsonNode body;

            public Builder body(JsonNode v) { this.body = v; return this; }

            public SpeechRequest build() { return new SpeechRequest(body); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof SpeechRequest)) return false;
            return Objects.equals(body, ((SpeechRequest) o).body);
        }

        @Override
        public int hashCode() { return Objects.hash(body); }

        @Override
        public String toString() { return "SpeechRequest(" + body + ")"; }
    }

    /** Provider response metadata for speech. */
    public static class SpeechResponse {
        @JsonProperty("timestamp") private String timestamp;
        @JsonProperty("model_id") private String modelId;
        @JsonProperty("headers") private Map<String, String> headers;
        @JsonProperty("body") private JsonNode body;

        @JsonCreator
        SpeechResponse() {}

        private SpeechResponse(String timestamp, String modelId,
                               Map<String, String> headers, JsonNode body) {
            this.timestamp = timestamp;
            this.modelId = modelId;
            this.headers = headers;
            this.body = body;
        }

        public static SpeechResponse of(String timestamp, String modelId,
                                        Map<String, String> headers, JsonNode body) {
            return new SpeechResponse(timestamp, modelId, headers, body);
        }

        public String getTimestamp() { return timestamp; }
        public String getModelId() { return modelId; }
        public Map<String, String> getHeaders() { return headers; }
        public JsonNode getBody() { return body; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String timestamp;
            private String modelId;
            private Map<String, String> headers;
            private JsonNode body;

            public Builder timestamp(String v) { this.timestamp = v; return this; }
            public Builder modelId(String v) { this.modelId = v; return this; }
            public Builder headers(Map<String, String> v) { this.headers = v; return this; }
            public Builder body(JsonNode v) { this.body = v; return this; }

            public SpeechResponse build() { return new SpeechResponse(timestamp, modelId, headers, body); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof SpeechResponse)) return false;
            SpeechResponse that = (SpeechResponse) o;
            return Objects.equals(timestamp, that.timestamp)
                && Objects.equals(modelId, that.modelId)
                && Objects.equals(headers, that.headers)
                && Objects.equals(body, that.body);
        }

        @Override
        public int hashCode() { return Objects.hash(timestamp, modelId, headers, body); }

        @Override
        public String toString() { return "SpeechResponse(" + timestamp + ", " + modelId + ", " + headers + ", " + body + ")"; }
    }

    /** Result of a speech generation call. */
    public static class SpeechResult {
        @JsonProperty("audio") private AudioData audio = new AudioData.Base64("");
        @JsonProperty("warnings") private List<JsonNode> warnings = new ArrayList<>();
        @JsonProperty("request") private SpeechRequest request;
        @JsonProperty("response") private SpeechResponse response = new SpeechResponse();
        @JsonProperty("provider_metadata") private JsonNode providerMetadata;

        @JsonCreator
        SpeechResult() {}

        private SpeechResult(AudioData audio, List<JsonNode> warnings, SpeechRequest request,
                             SpeechResponse response, JsonNode providerMetadata) {
            this.audio = audio;
            this.warnings = warnings;
            this.request = request;
            this.response = response;
            this.providerMetadata = providerMetadata;
        }

        public static SpeechResult of(AudioData audio) {
            return new SpeechResult(audio, new ArrayList<JsonNode>(), null, new SpeechResponse(), null);
        }

        public AudioData getAudio() { return audio; }
        public List<JsonNode> getWarnings() { return warnings; }
        public SpeechRequest getRequest() { return request; }
        public SpeechResponse getResponse() { return response; }
        public JsonNode getProviderMetadata() { return providerMetadata; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private AudioData audio = new AudioData.Base64("");
            private List<JsonNode> warnings = new ArrayList<>();
            private SpeechRequest request;
            private SpeechResponse response = new SpeechResponse();
            private JsonNode providerMetadata;

            public Builder audio(AudioData v) { this.audio = v; return this; }
            public Builder warnings(List<JsonNode> v) { this.warnings = v; return this; }
            public Builder request(SpeechRequest v) { this.request = v; return this; }
            public Builder response(SpeechResponse v) { this.response = v; return this; }
            public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

            public SpeechResult build() {
                return new SpeechResult(audio, warnings, request, response, providerMetadata);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof SpeechResult)) return false;
            SpeechResult that = (SpeechResult) o;
            return Objects.equals(audio, that.audio)
                && Objects.equals(warnings, that.warnings)
                && Objects.equals(request, that.request)
                && Objects.equals(response, that.response)
                && Objects.equals(providerMetadata, that.providerMetadata);
        }

        @Override
        public int hashCode() { return Objects.hash(audio, warnings, request, response, providerMetadata); }

        @Override
        public String toString() { return "SpeechResult(" + audio + ")"; }
    }

    /** Options for speech generation. */
    public static class SpeechCallOptions {
        @JsonProperty("text") private String text = "";
        @JsonProperty("voice") private String voice;
        @JsonProperty("output_format") private String outputFormat;
        @JsonProperty("instructions") private String instructions;
        @JsonProperty("speed") private Double speed;
        @JsonProperty("language") private String language;
        @JsonProperty("provider_options") private JsonNode providerOptions;
        @JsonProperty("headers") private Map<String, String> headers;

        @JsonCreator
        SpeechCallOptions() {}

        private SpeechCallOptions(String text, String voice, String outputFormat, String instructions,
                                  Double speed, String language, JsonNode providerOptions,
                                  Map<String, String> headers) {
            this.text = text;
            this.voice = voice;
            this.outputFormat = outputFormat;
            this.instructions = instructions;
            this.speed = speed;
            this.language = language;
            this.providerOptions = providerOptions;
            this.headers = headers;
        }

        public static SpeechCallOptions of(String text) {
            return new SpeechCallOptions(text, null, null, null, null, null, null, null);
        }

        public String getText() { return text; }
        public String getVoice() { return voice; }
        public String getOutputFormat() { return outputFormat; }
        public String getInstructions() { return instructions; }
        public Double getSpeed() { return speed; }
        public String getLanguage() { return language; }
        public JsonNode getProviderOptions() { return providerOptions; }
        public Map<String, String> getHeaders() { return headers; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String text = "";
            private String voice;
            private String outputFormat;
            private String instructions;
            private Double speed;
            private String language;
            private JsonNode providerOptions;
            private Map<String, String> headers;

            public Builder text(String v) { this.text = v; return this; }
            public Builder voice(String v) { this.voice = v; return this; }
            public Builder outputFormat(String v) { this.outputFormat = v; return this; }
            public Builder instructions(String v) { this.instructions = v; return this; }
            public Builder speed(Double v) { this.speed = v; return this; }
            public Builder language(String v) { this.language = v; return this; }
            public Builder providerOptions(JsonNode v) { this.providerOptions = v; return this; }
            public Builder headers(Map<String, String> v) { this.headers = v; return this; }

            public SpeechCallOptions build() {
                return new SpeechCallOptions(text, voice, outputFormat, instructions,
                    speed, language, providerOptions, headers);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof SpeechCallOptions)) return false;
            SpeechCallOptions that = (SpeechCallOptions) o;
            return Objects.equals(text, that.text)
                && Objects.equals(voice, that.voice)
                && Objects.equals(outputFormat, that.outputFormat)
                && Objects.equals(instructions, that.instructions)
                && Objects.equals(speed, that.speed)
                && Objects.equals(language, that.language)
                && Objects.equals(providerOptions, that.providerOptions)
                && Objects.equals(headers, that.headers);
        }

        @Override
        public int hashCode() {
            return Objects.hash(text, voice, outputFormat, instructions, speed, language, providerOptions, headers);
        }

        @Override
        public String toString() { return "SpeechCallOptions(" + text + ")"; }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Image
    // ─────────────────────────────────────────────────────────────────────────────

    /**
     * Generated images: all base64 strings or all binary byte arrays.
     *
     * <p>Wire format (serde externally-tagged):
     * {@code {"Base64": ["...", ...]}} | {@code {"Binary": [[n,...], ...]}}.
     */
    public abstract static class ImageOutputs {
        private ImageOutputs() {}

        /** Base64-encoded images. */
        public static class Base64 extends ImageOutputs {
            private List<String> value = new ArrayList<>();

            @JsonCreator
            Base64() {}

            public Base64(List<String> value) { this.value = value; }

            public List<String> getValue() { return value; }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Base64)) return false;
                return Objects.equals(value, ((Base64) o).value);
            }

            @Override
            public int hashCode() { return Objects.hash(value); }

            @Override
            public String toString() { return "ImageOutputs.Base64(" + value + ")"; }
        }

        /** Raw binary images (each element is a list of 0–255 byte values). */
        public static class Binary extends ImageOutputs {
            private List<List<Integer>> value = new ArrayList<>();

            @JsonCreator
            Binary() {}

            public Binary(List<List<Integer>> value) { this.value = value; }

            public List<List<Integer>> getValue() { return value; }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Binary)) return false;
                return Objects.equals(value, ((Binary) o).value);
            }

            @Override
            public int hashCode() { return Objects.hash(value); }

            @Override
            public String toString() { return "ImageOutputs.Binary(" + value + ")"; }
        }
    }

    /** Custom (de)serializer for {@link ImageOutputs}: {@code {"Base64": [...]}} | {@code {"Binary": [[...],...]}}. */
    public static class ImageOutputsSerializer extends JsonSerializer<ImageOutputs> {
        @Override
        public void serialize(ImageOutputs value, JsonGenerator gen, SerializerProvider serializers) throws IOException {
            gen.writeStartObject();
            if (value instanceof ImageOutputs.Base64) {
                gen.writeArrayFieldStart("Base64");
                for (String s : ((ImageOutputs.Base64) value).getValue()) {
                    gen.writeString(s);
                }
                gen.writeEndArray();
            } else if (value instanceof ImageOutputs.Binary) {
                gen.writeArrayFieldStart("Binary");
                for (List<Integer> row : ((ImageOutputs.Binary) value).getValue()) {
                    gen.writeStartArray();
                    for (Integer b : row) {
                        gen.writeNumber(b);
                    }
                    gen.writeEndArray();
                }
                gen.writeEndArray();
            } else {
                throw new IOException("Unknown ImageOutputs: " + value);
            }
            gen.writeEndObject();
        }
    }

    /** Custom (de)serializer for {@link ImageOutputs}. */
    public static class ImageOutputsDeserializer extends JsonDeserializer<ImageOutputs> {
        @Override
        public ImageOutputs deserialize(JsonParser p, DeserializationContext ctxt) throws IOException {
            JsonNode node = p.getCodec().readTree(p);
            if (!node.isObject() || node.size() != 1) {
                throw new IOException("ImageOutputs must be a single-key externally-tagged object, got: " + node);
            }
            String tag = node.fieldNames().next();
            JsonNode inner = node.get(tag);
            switch (tag) {
                case "Base64": {
                    List<String> values = new ArrayList<>();
                    if (inner.isArray()) {
                        for (JsonNode item : inner) {
                            values.add(item.asText());
                        }
                    }
                    return new ImageOutputs.Base64(values);
                }
                case "Binary": {
                    List<List<Integer>> rows = new ArrayList<>();
                    if (inner.isArray()) {
                        for (JsonNode rowNode : inner) {
                            List<Integer> row = new ArrayList<>();
                            if (rowNode.isArray()) {
                                for (JsonNode item : rowNode) {
                                    row.add(item.asInt());
                                }
                            }
                            rows.add(row);
                        }
                    }
                    return new ImageOutputs.Binary(rows);
                }
                default:
                    throw new IOException("Unknown ImageOutputs tag: '" + tag + "'");
            }
        }
    }

    /** Token usage for image generation (if reported). */
    public static class ImageUsage {
        @JsonProperty("input_tokens") private Long inputTokens;
        @JsonProperty("output_tokens") private Long outputTokens;
        @JsonProperty("total_tokens") private Long totalTokens;

        @JsonCreator
        ImageUsage() {}

        private ImageUsage(Long inputTokens, Long outputTokens, Long totalTokens) {
            this.inputTokens = inputTokens;
            this.outputTokens = outputTokens;
            this.totalTokens = totalTokens;
        }

        public static ImageUsage of(Long inputTokens, Long outputTokens, Long totalTokens) {
            return new ImageUsage(inputTokens, outputTokens, totalTokens);
        }

        public Long getInputTokens() { return inputTokens; }
        public Long getOutputTokens() { return outputTokens; }
        public Long getTotalTokens() { return totalTokens; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private Long inputTokens;
            private Long outputTokens;
            private Long totalTokens;

            public Builder inputTokens(Long v) { this.inputTokens = v; return this; }
            public Builder outputTokens(Long v) { this.outputTokens = v; return this; }
            public Builder totalTokens(Long v) { this.totalTokens = v; return this; }

            public ImageUsage build() { return new ImageUsage(inputTokens, outputTokens, totalTokens); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof ImageUsage)) return false;
            ImageUsage that = (ImageUsage) o;
            return Objects.equals(inputTokens, that.inputTokens)
                && Objects.equals(outputTokens, that.outputTokens)
                && Objects.equals(totalTokens, that.totalTokens);
        }

        @Override
        public int hashCode() { return Objects.hash(inputTokens, outputTokens, totalTokens); }

        @Override
        public String toString() { return "ImageUsage(" + inputTokens + ", " + outputTokens + ", " + totalTokens + ")"; }
    }

    /** Provider response metadata for images. */
    public static class ImageResponse {
        @JsonProperty("timestamp") private String timestamp;
        @JsonProperty("model_id") private String modelId;
        @JsonProperty("headers") private Map<String, String> headers;

        @JsonCreator
        ImageResponse() {}

        private ImageResponse(String timestamp, String modelId, Map<String, String> headers) {
            this.timestamp = timestamp;
            this.modelId = modelId;
            this.headers = headers;
        }

        public static ImageResponse of(String timestamp, String modelId, Map<String, String> headers) {
            return new ImageResponse(timestamp, modelId, headers);
        }

        public String getTimestamp() { return timestamp; }
        public String getModelId() { return modelId; }
        public Map<String, String> getHeaders() { return headers; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String timestamp;
            private String modelId;
            private Map<String, String> headers;

            public Builder timestamp(String v) { this.timestamp = v; return this; }
            public Builder modelId(String v) { this.modelId = v; return this; }
            public Builder headers(Map<String, String> v) { this.headers = v; return this; }

            public ImageResponse build() { return new ImageResponse(timestamp, modelId, headers); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof ImageResponse)) return false;
            ImageResponse that = (ImageResponse) o;
            return Objects.equals(timestamp, that.timestamp)
                && Objects.equals(modelId, that.modelId)
                && Objects.equals(headers, that.headers);
        }

        @Override
        public int hashCode() { return Objects.hash(timestamp, modelId, headers); }

        @Override
        public String toString() { return "ImageResponse(" + timestamp + ", " + modelId + ", " + headers + ")"; }
    }

    /** Result of an image generation call. */
    public static class ImageResult {
        @JsonProperty("images") private ImageOutputs images = new ImageOutputs.Base64(new ArrayList<String>());
        @JsonProperty("warnings") private List<JsonNode> warnings = new ArrayList<>();
        @JsonProperty("provider_metadata") private JsonNode providerMetadata;
        @JsonProperty("response") private ImageResponse response = new ImageResponse();
        @JsonProperty("usage") private ImageUsage usage;

        @JsonCreator
        ImageResult() {}

        private ImageResult(ImageOutputs images, List<JsonNode> warnings, JsonNode providerMetadata,
                            ImageResponse response, ImageUsage usage) {
            this.images = images;
            this.warnings = warnings;
            this.providerMetadata = providerMetadata;
            this.response = response;
            this.usage = usage;
        }

        public static ImageResult of(ImageOutputs images) {
            return new ImageResult(images, new ArrayList<JsonNode>(), null, new ImageResponse(), null);
        }

        public ImageOutputs getImages() { return images; }
        public List<JsonNode> getWarnings() { return warnings; }
        public JsonNode getProviderMetadata() { return providerMetadata; }
        public ImageResponse getResponse() { return response; }
        public ImageUsage getUsage() { return usage; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private ImageOutputs images = new ImageOutputs.Base64(new ArrayList<String>());
            private List<JsonNode> warnings = new ArrayList<>();
            private JsonNode providerMetadata;
            private ImageResponse response = new ImageResponse();
            private ImageUsage usage;

            public Builder images(ImageOutputs v) { this.images = v; return this; }
            public Builder warnings(List<JsonNode> v) { this.warnings = v; return this; }
            public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }
            public Builder response(ImageResponse v) { this.response = v; return this; }
            public Builder usage(ImageUsage v) { this.usage = v; return this; }

            public ImageResult build() {
                return new ImageResult(images, warnings, providerMetadata, response, usage);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof ImageResult)) return false;
            ImageResult that = (ImageResult) o;
            return Objects.equals(images, that.images)
                && Objects.equals(warnings, that.warnings)
                && Objects.equals(providerMetadata, that.providerMetadata)
                && Objects.equals(response, that.response)
                && Objects.equals(usage, that.usage);
        }

        @Override
        public int hashCode() { return Objects.hash(images, warnings, providerMetadata, response, usage); }

        @Override
        public String toString() { return "ImageResult(" + images + ")"; }
    }

    /** Options for image generation. */
    public static class ImageCallOptions {
        @JsonProperty("prompt") private String prompt;
        @JsonProperty("n") private Integer n;
        @JsonProperty("size") private String size;
        @JsonProperty("aspect_ratio") private String aspectRatio;
        @JsonProperty("seed") private Long seed;
        @JsonProperty("files") private List<JsonNode> files = new ArrayList<>();
        @JsonProperty("mask") private JsonNode mask;
        @JsonProperty("provider_options") private JsonNode providerOptions;
        @JsonProperty("headers") private Map<String, String> headers;

        @JsonCreator
        ImageCallOptions() {}

        private ImageCallOptions(String prompt, Integer n, String size, String aspectRatio,
                                 Long seed, List<JsonNode> files, JsonNode mask,
                                 JsonNode providerOptions, Map<String, String> headers) {
            this.prompt = prompt;
            this.n = n;
            this.size = size;
            this.aspectRatio = aspectRatio;
            this.seed = seed;
            this.files = files;
            this.mask = mask;
            this.providerOptions = providerOptions;
            this.headers = headers;
        }

        public static ImageCallOptions of(String prompt) {
            return new ImageCallOptions(prompt, null, null, null, null, new ArrayList<JsonNode>(),
                null, null, null);
        }

        public String getPrompt() { return prompt; }
        public Integer getN() { return n; }
        public String getSize() { return size; }
        public String getAspectRatio() { return aspectRatio; }
        public Long getSeed() { return seed; }
        public List<JsonNode> getFiles() { return files; }
        public JsonNode getMask() { return mask; }
        public JsonNode getProviderOptions() { return providerOptions; }
        public Map<String, String> getHeaders() { return headers; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String prompt;
            private Integer n;
            private String size;
            private String aspectRatio;
            private Long seed;
            private List<JsonNode> files = new ArrayList<>();
            private JsonNode mask;
            private JsonNode providerOptions;
            private Map<String, String> headers;

            public Builder prompt(String v) { this.prompt = v; return this; }
            public Builder n(Integer v) { this.n = v; return this; }
            public Builder size(String v) { this.size = v; return this; }
            public Builder aspectRatio(String v) { this.aspectRatio = v; return this; }
            public Builder seed(Long v) { this.seed = v; return this; }
            public Builder files(List<JsonNode> v) { this.files = v; return this; }
            public Builder mask(JsonNode v) { this.mask = v; return this; }
            public Builder providerOptions(JsonNode v) { this.providerOptions = v; return this; }
            public Builder headers(Map<String, String> v) { this.headers = v; return this; }

            public ImageCallOptions build() {
                return new ImageCallOptions(prompt, n, size, aspectRatio, seed, files, mask,
                    providerOptions, headers);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof ImageCallOptions)) return false;
            ImageCallOptions that = (ImageCallOptions) o;
            return Objects.equals(prompt, that.prompt)
                && Objects.equals(n, that.n)
                && Objects.equals(size, that.size)
                && Objects.equals(aspectRatio, that.aspectRatio)
                && Objects.equals(seed, that.seed)
                && Objects.equals(files, that.files)
                && Objects.equals(mask, that.mask)
                && Objects.equals(providerOptions, that.providerOptions)
                && Objects.equals(headers, that.headers);
        }

        @Override
        public int hashCode() {
            return Objects.hash(prompt, n, size, aspectRatio, seed, files, mask, providerOptions, headers);
        }

        @Override
        public String toString() { return "ImageCallOptions(" + prompt + ")"; }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Transcription (STT)
    // ─────────────────────────────────────────────────────────────────────────────

    /** A transcript segment with timing. */
    public static class TranscriptionSegment {
        @JsonProperty("text") private String text = "";
        // Core declares these as required f64 (TranscriptionSegment.ts), so they
        // are primitives: NON_NULL inclusion would drop boxed nulls and the core
        // would reject the payload with `missing field start_second`.
        @JsonProperty("start_second") private double startSecond;
        @JsonProperty("end_second") private double endSecond;

        @JsonCreator
        TranscriptionSegment() {}

        private TranscriptionSegment(String text, double startSecond, double endSecond) {
            this.text = text;
            this.startSecond = startSecond;
            this.endSecond = endSecond;
        }

        public static TranscriptionSegment of(String text, double startSecond, double endSecond) {
            return new TranscriptionSegment(text, startSecond, endSecond);
        }

        public String getText() { return text; }
        public double getStartSecond() { return startSecond; }
        public double getEndSecond() { return endSecond; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String text = "";
            private double startSecond;
            private double endSecond;

            public Builder text(String v) { this.text = v; return this; }
            public Builder startSecond(double v) { this.startSecond = v; return this; }
            public Builder endSecond(double v) { this.endSecond = v; return this; }

            public TranscriptionSegment build() { return new TranscriptionSegment(text, startSecond, endSecond); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof TranscriptionSegment)) return false;
            TranscriptionSegment that = (TranscriptionSegment) o;
            return Objects.equals(text, that.text)
                && Double.compare(startSecond, that.startSecond) == 0
                && Double.compare(endSecond, that.endSecond) == 0;
        }

        @Override
        public int hashCode() { return Objects.hash(text, startSecond, endSecond); }

        @Override
        public String toString() { return "TranscriptionSegment(" + text + ")"; }
    }

    /** Request metadata for transcription. */
    public static class TranscriptionRequest {
        // Core models this as `body: Option<String>` (transcription_model.rs)
        // — the raw request HTTP body, JSON stringified.
        @JsonProperty("body") private String body;

        @JsonCreator
        TranscriptionRequest() {}

        private TranscriptionRequest(String body) {
            this.body = body;
        }

        public static TranscriptionRequest of(String body) {
            return new TranscriptionRequest(body);
        }

        public String getBody() { return body; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String body;

            public Builder body(String v) { this.body = v; return this; }

            public TranscriptionRequest build() { return new TranscriptionRequest(body); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof TranscriptionRequest)) return false;
            return Objects.equals(body, ((TranscriptionRequest) o).body);
        }

        @Override
        public int hashCode() { return Objects.hash(body); }

        @Override
        public String toString() { return "TranscriptionRequest(" + body + ")"; }
    }

    /** Provider response metadata for transcription. */
    public static class TranscriptionResponse {
        @JsonProperty("timestamp") private String timestamp;
        @JsonProperty("model_id") private String modelId;
        @JsonProperty("headers") private Map<String, String> headers;
        @JsonProperty("body") private JsonNode body;

        @JsonCreator
        TranscriptionResponse() {}

        private TranscriptionResponse(String timestamp, String modelId,
                                      Map<String, String> headers, JsonNode body) {
            this.timestamp = timestamp;
            this.modelId = modelId;
            this.headers = headers;
            this.body = body;
        }

        public static TranscriptionResponse of(String timestamp, String modelId,
                                               Map<String, String> headers, JsonNode body) {
            return new TranscriptionResponse(timestamp, modelId, headers, body);
        }

        public String getTimestamp() { return timestamp; }
        public String getModelId() { return modelId; }
        public Map<String, String> getHeaders() { return headers; }
        public JsonNode getBody() { return body; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String timestamp;
            private String modelId;
            private Map<String, String> headers;
            private JsonNode body;

            public Builder timestamp(String v) { this.timestamp = v; return this; }
            public Builder modelId(String v) { this.modelId = v; return this; }
            public Builder headers(Map<String, String> v) { this.headers = v; return this; }
            public Builder body(JsonNode v) { this.body = v; return this; }

            public TranscriptionResponse build() { return new TranscriptionResponse(timestamp, modelId, headers, body); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof TranscriptionResponse)) return false;
            TranscriptionResponse that = (TranscriptionResponse) o;
            return Objects.equals(timestamp, that.timestamp)
                && Objects.equals(modelId, that.modelId)
                && Objects.equals(headers, that.headers)
                && Objects.equals(body, that.body);
        }

        @Override
        public int hashCode() { return Objects.hash(timestamp, modelId, headers, body); }

        @Override
        public String toString() { return "TranscriptionResponse(" + timestamp + ", " + modelId + ", " + headers + ", " + body + ")"; }
    }

    /** Result of a transcription call. */
    public static class TranscriptionResult {
        @JsonProperty("text") private String text = "";
        @JsonProperty("segments") private List<TranscriptionSegment> segments = new ArrayList<>();
        @JsonProperty("language") private String language;
        @JsonProperty("duration_in_seconds") private Double durationInSeconds;
        @JsonProperty("warnings") private List<JsonNode> warnings = new ArrayList<>();
        @JsonProperty("request") private TranscriptionRequest request;
        @JsonProperty("response") private TranscriptionResponse response = new TranscriptionResponse();
        @JsonProperty("provider_metadata") private JsonNode providerMetadata;

        @JsonCreator
        TranscriptionResult() {}

        private TranscriptionResult(String text, List<TranscriptionSegment> segments, String language,
                                    Double durationInSeconds, List<JsonNode> warnings,
                                    TranscriptionRequest request, TranscriptionResponse response,
                                    JsonNode providerMetadata) {
            this.text = text;
            this.segments = segments;
            this.language = language;
            this.durationInSeconds = durationInSeconds;
            this.warnings = warnings;
            this.request = request;
            this.response = response;
            this.providerMetadata = providerMetadata;
        }

        public static TranscriptionResult of(String text) {
            return new TranscriptionResult(text, new ArrayList<TranscriptionSegment>(), null, null,
                new ArrayList<JsonNode>(), null, new TranscriptionResponse(), null);
        }

        public String getText() { return text; }
        public List<TranscriptionSegment> getSegments() { return segments; }
        public String getLanguage() { return language; }
        public Double getDurationInSeconds() { return durationInSeconds; }
        public List<JsonNode> getWarnings() { return warnings; }
        public TranscriptionRequest getRequest() { return request; }
        public TranscriptionResponse getResponse() { return response; }
        public JsonNode getProviderMetadata() { return providerMetadata; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String text = "";
            private List<TranscriptionSegment> segments = new ArrayList<>();
            private String language;
            private Double durationInSeconds;
            private List<JsonNode> warnings = new ArrayList<>();
            private TranscriptionRequest request;
            private TranscriptionResponse response = new TranscriptionResponse();
            private JsonNode providerMetadata;

            public Builder text(String v) { this.text = v; return this; }
            public Builder segments(List<TranscriptionSegment> v) { this.segments = v; return this; }
            public Builder language(String v) { this.language = v; return this; }
            public Builder durationInSeconds(Double v) { this.durationInSeconds = v; return this; }
            public Builder warnings(List<JsonNode> v) { this.warnings = v; return this; }
            public Builder request(TranscriptionRequest v) { this.request = v; return this; }
            public Builder response(TranscriptionResponse v) { this.response = v; return this; }
            public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

            public TranscriptionResult build() {
                return new TranscriptionResult(text, segments, language, durationInSeconds,
                    warnings, request, response, providerMetadata);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof TranscriptionResult)) return false;
            TranscriptionResult that = (TranscriptionResult) o;
            return Objects.equals(text, that.text)
                && Objects.equals(segments, that.segments)
                && Objects.equals(language, that.language)
                && Objects.equals(durationInSeconds, that.durationInSeconds)
                && Objects.equals(warnings, that.warnings)
                && Objects.equals(request, that.request)
                && Objects.equals(response, that.response)
                && Objects.equals(providerMetadata, that.providerMetadata);
        }

        @Override
        public int hashCode() {
            return Objects.hash(text, segments, language, durationInSeconds, warnings, request, response, providerMetadata);
        }

        @Override
        public String toString() { return "TranscriptionResult(" + text + ")"; }
    }

    /** Options for transcription. */
    public static class TranscriptionCallOptions {
        @JsonProperty("audio") private JsonNode audio = JsonNodeFactory.instance.objectNode();
        @JsonProperty("media_type") private String mediaType = "";
        @JsonProperty("provider_options") private JsonNode providerOptions;
        @JsonProperty("headers") private Map<String, String> headers;

        @JsonCreator
        TranscriptionCallOptions() {}

        private TranscriptionCallOptions(JsonNode audio, String mediaType,
                                         JsonNode providerOptions, Map<String, String> headers) {
            this.audio = audio;
            this.mediaType = mediaType;
            this.providerOptions = providerOptions;
            this.headers = headers;
        }

        public static TranscriptionCallOptions of(JsonNode audio, String mediaType) {
            return new TranscriptionCallOptions(audio, mediaType, null, null);
        }

        public JsonNode getAudio() { return audio; }
        public String getMediaType() { return mediaType; }
        public JsonNode getProviderOptions() { return providerOptions; }
        public Map<String, String> getHeaders() { return headers; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private JsonNode audio = JsonNodeFactory.instance.objectNode();
            private String mediaType = "";
            private JsonNode providerOptions;
            private Map<String, String> headers;

            public Builder audio(JsonNode v) { this.audio = v; return this; }
            public Builder mediaType(String v) { this.mediaType = v; return this; }
            public Builder providerOptions(JsonNode v) { this.providerOptions = v; return this; }
            public Builder headers(Map<String, String> v) { this.headers = v; return this; }

            public TranscriptionCallOptions build() {
                return new TranscriptionCallOptions(audio, mediaType, providerOptions, headers);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof TranscriptionCallOptions)) return false;
            TranscriptionCallOptions that = (TranscriptionCallOptions) o;
            return Objects.equals(audio, that.audio)
                && Objects.equals(mediaType, that.mediaType)
                && Objects.equals(providerOptions, that.providerOptions)
                && Objects.equals(headers, that.headers);
        }

        @Override
        public int hashCode() { return Objects.hash(audio, mediaType, providerOptions, headers); }

        @Override
        public String toString() { return "TranscriptionCallOptions(" + mediaType + ")"; }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Reranking
    // ─────────────────────────────────────────────────────────────────────────────

    /** A single reranked entry. */
    public static class RerankingRank {
        @JsonProperty("index") private int index;
        @JsonProperty("relevance_score") private double relevanceScore;

        @JsonCreator
        RerankingRank() {}

        private RerankingRank(int index, double relevanceScore) {
            this.index = index;
            this.relevanceScore = relevanceScore;
        }

        public static RerankingRank of(int index, double relevanceScore) {
            return new RerankingRank(index, relevanceScore);
        }

        public int getIndex() { return index; }
        public double getRelevanceScore() { return relevanceScore; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private int index;
            private double relevanceScore;

            public Builder index(int v) { this.index = v; return this; }
            public Builder relevanceScore(double v) { this.relevanceScore = v; return this; }

            public RerankingRank build() { return new RerankingRank(index, relevanceScore); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof RerankingRank)) return false;
            RerankingRank that = (RerankingRank) o;
            return index == that.index
                && Double.compare(relevanceScore, that.relevanceScore) == 0;
        }

        @Override
        public int hashCode() { return Objects.hash(index, relevanceScore); }

        @Override
        public String toString() { return "RerankingRank(" + index + ", " + relevanceScore + ")"; }
    }

    /** Provider response metadata for reranking. */
    public static class RerankingResponse {
        @JsonProperty("id") private String id;
        @JsonProperty("timestamp") private String timestamp;
        @JsonProperty("model_id") private String modelId;
        @JsonProperty("headers") private Map<String, String> headers;
        @JsonProperty("body") private JsonNode body;

        @JsonCreator
        RerankingResponse() {}

        private RerankingResponse(String id, String timestamp, String modelId,
                                  Map<String, String> headers, JsonNode body) {
            this.id = id;
            this.timestamp = timestamp;
            this.modelId = modelId;
            this.headers = headers;
            this.body = body;
        }

        public static RerankingResponse of(String id, String timestamp, String modelId,
                                           Map<String, String> headers, JsonNode body) {
            return new RerankingResponse(id, timestamp, modelId, headers, body);
        }

        public String getId() { return id; }
        public String getTimestamp() { return timestamp; }
        public String getModelId() { return modelId; }
        public Map<String, String> getHeaders() { return headers; }
        public JsonNode getBody() { return body; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String id;
            private String timestamp;
            private String modelId;
            private Map<String, String> headers;
            private JsonNode body;

            public Builder id(String v) { this.id = v; return this; }
            public Builder timestamp(String v) { this.timestamp = v; return this; }
            public Builder modelId(String v) { this.modelId = v; return this; }
            public Builder headers(Map<String, String> v) { this.headers = v; return this; }
            public Builder body(JsonNode v) { this.body = v; return this; }

            public RerankingResponse build() { return new RerankingResponse(id, timestamp, modelId, headers, body); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof RerankingResponse)) return false;
            RerankingResponse that = (RerankingResponse) o;
            return Objects.equals(id, that.id)
                && Objects.equals(timestamp, that.timestamp)
                && Objects.equals(modelId, that.modelId)
                && Objects.equals(headers, that.headers)
                && Objects.equals(body, that.body);
        }

        @Override
        public int hashCode() { return Objects.hash(id, timestamp, modelId, headers, body); }

        @Override
        public String toString() { return "RerankingResponse(" + id + ", " + timestamp + ", " + modelId + ", " + headers + ", " + body + ")"; }
    }

    /** Result of a reranking call. */
    public static class RerankingResult {
        @JsonProperty("ranking") private List<RerankingRank> ranking = new ArrayList<>();
        @JsonProperty("provider_metadata") private JsonNode providerMetadata;
        @JsonProperty("warnings") private List<JsonNode> warnings = new ArrayList<>();
        @JsonProperty("response") private RerankingResponse response;

        @JsonCreator
        RerankingResult() {}

        private RerankingResult(List<RerankingRank> ranking, JsonNode providerMetadata,
                                List<JsonNode> warnings, RerankingResponse response) {
            this.ranking = ranking;
            this.providerMetadata = providerMetadata;
            this.warnings = warnings;
            this.response = response;
        }

        public static RerankingResult of(List<RerankingRank> ranking) {
            return new RerankingResult(ranking, null, new ArrayList<JsonNode>(), null);
        }

        public List<RerankingRank> getRanking() { return ranking; }
        public JsonNode getProviderMetadata() { return providerMetadata; }
        public List<JsonNode> getWarnings() { return warnings; }
        public RerankingResponse getResponse() { return response; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private List<RerankingRank> ranking = new ArrayList<>();
            private JsonNode providerMetadata;
            private List<JsonNode> warnings = new ArrayList<>();
            private RerankingResponse response;

            public Builder ranking(List<RerankingRank> v) { this.ranking = v; return this; }
            public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }
            public Builder warnings(List<JsonNode> v) { this.warnings = v; return this; }
            public Builder response(RerankingResponse v) { this.response = v; return this; }

            public RerankingResult build() {
                return new RerankingResult(ranking, providerMetadata, warnings, response);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof RerankingResult)) return false;
            RerankingResult that = (RerankingResult) o;
            return Objects.equals(ranking, that.ranking)
                && Objects.equals(providerMetadata, that.providerMetadata)
                && Objects.equals(warnings, that.warnings)
                && Objects.equals(response, that.response);
        }

        @Override
        public int hashCode() { return Objects.hash(ranking, providerMetadata, warnings, response); }

        @Override
        public String toString() { return "RerankingResult(" + ranking + ")"; }
    }

    /** Options for reranking. */
    public static class RerankingCallOptions {
        @JsonProperty("documents") private JsonNode documents = JsonNodeFactory.instance.arrayNode();
        @JsonProperty("query") private String query = "";
        @JsonProperty("top_n") private Integer topN;
        @JsonProperty("provider_options") private JsonNode providerOptions;
        @JsonProperty("headers") private Map<String, String> headers;

        @JsonCreator
        RerankingCallOptions() {}

        private RerankingCallOptions(JsonNode documents, String query, Integer topN,
                                     JsonNode providerOptions, Map<String, String> headers) {
            this.documents = documents;
            this.query = query;
            this.topN = topN;
            this.providerOptions = providerOptions;
            this.headers = headers;
        }

        public static RerankingCallOptions of(JsonNode documents, String query) {
            return new RerankingCallOptions(documents, query, null, null, null);
        }

        public JsonNode getDocuments() { return documents; }
        public String getQuery() { return query; }
        public Integer getTopN() { return topN; }
        public JsonNode getProviderOptions() { return providerOptions; }
        public Map<String, String> getHeaders() { return headers; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private JsonNode documents = JsonNodeFactory.instance.arrayNode();
            private String query = "";
            private Integer topN;
            private JsonNode providerOptions;
            private Map<String, String> headers;

            public Builder documents(JsonNode v) { this.documents = v; return this; }
            public Builder query(String v) { this.query = v; return this; }
            public Builder topN(Integer v) { this.topN = v; return this; }
            public Builder providerOptions(JsonNode v) { this.providerOptions = v; return this; }
            public Builder headers(Map<String, String> v) { this.headers = v; return this; }

            public RerankingCallOptions build() {
                return new RerankingCallOptions(documents, query, topN, providerOptions, headers);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof RerankingCallOptions)) return false;
            RerankingCallOptions that = (RerankingCallOptions) o;
            return Objects.equals(documents, that.documents)
                && Objects.equals(query, that.query)
                && Objects.equals(topN, that.topN)
                && Objects.equals(providerOptions, that.providerOptions)
                && Objects.equals(headers, that.headers);
        }

        @Override
        public int hashCode() { return Objects.hash(documents, query, topN, providerOptions, headers); }

        @Override
        public String toString() { return "RerankingCallOptions(" + query + ")"; }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Video
    // ─────────────────────────────────────────────────────────────────────────────

    /** The URL variant of {@link VideoData}. */
    public static class VideoUrlData {
        @JsonProperty("url") private String url = "";
        @JsonProperty("media_type") private String mediaType = "";

        @JsonCreator
        VideoUrlData() {}

        private VideoUrlData(String url, String mediaType) {
            this.url = url;
            this.mediaType = mediaType;
        }

        public static VideoUrlData of(String url, String mediaType) {
            return new VideoUrlData(url, mediaType);
        }

        public String getUrl() { return url; }
        public String getMediaType() { return mediaType; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String url = "";
            private String mediaType = "";

            public Builder url(String v) { this.url = v; return this; }
            public Builder mediaType(String v) { this.mediaType = v; return this; }

            public VideoUrlData build() { return new VideoUrlData(url, mediaType); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof VideoUrlData)) return false;
            VideoUrlData that = (VideoUrlData) o;
            return Objects.equals(url, that.url)
                && Objects.equals(mediaType, that.mediaType);
        }

        @Override
        public int hashCode() { return Objects.hash(url, mediaType); }

        @Override
        public String toString() { return "VideoUrlData(" + url + ", " + mediaType + ")"; }
    }

    /** The base64 variant of {@link VideoData}. */
    public static class VideoBase64Data {
        @JsonProperty("data") private String data = "";
        @JsonProperty("media_type") private String mediaType = "";

        @JsonCreator
        VideoBase64Data() {}

        private VideoBase64Data(String data, String mediaType) {
            this.data = data;
            this.mediaType = mediaType;
        }

        public static VideoBase64Data of(String data, String mediaType) {
            return new VideoBase64Data(data, mediaType);
        }

        public String getData() { return data; }
        public String getMediaType() { return mediaType; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String data = "";
            private String mediaType = "";

            public Builder data(String v) { this.data = v; return this; }
            public Builder mediaType(String v) { this.mediaType = v; return this; }

            public VideoBase64Data build() { return new VideoBase64Data(data, mediaType); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof VideoBase64Data)) return false;
            VideoBase64Data that = (VideoBase64Data) o;
            return Objects.equals(data, that.data)
                && Objects.equals(mediaType, that.mediaType);
        }

        @Override
        public int hashCode() { return Objects.hash(data, mediaType); }

        @Override
        public String toString() { return "VideoBase64Data(" + data + ", " + mediaType + ")"; }
    }

    /** The binary variant of {@link VideoData}. */
    public static class VideoBinaryData {
        @JsonProperty("data") private List<Integer> data = new ArrayList<>();
        @JsonProperty("media_type") private String mediaType = "";

        @JsonCreator
        VideoBinaryData() {}

        private VideoBinaryData(List<Integer> data, String mediaType) {
            this.data = data;
            this.mediaType = mediaType;
        }

        public static VideoBinaryData of(List<Integer> data, String mediaType) {
            return new VideoBinaryData(data, mediaType);
        }

        public List<Integer> getData() { return data; }
        public String getMediaType() { return mediaType; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private List<Integer> data = new ArrayList<>();
            private String mediaType = "";

            public Builder data(List<Integer> v) { this.data = v; return this; }
            public Builder mediaType(String v) { this.mediaType = v; return this; }

            public VideoBinaryData build() { return new VideoBinaryData(data, mediaType); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof VideoBinaryData)) return false;
            VideoBinaryData that = (VideoBinaryData) o;
            return Objects.equals(data, that.data)
                && Objects.equals(mediaType, that.mediaType);
        }

        @Override
        public int hashCode() { return Objects.hash(data, mediaType); }

        @Override
        public String toString() { return "VideoBinaryData(" + mediaType + ")"; }
    }

    /**
     * Generated video: a URL, base64 string, or raw binary bytes.
     *
     * <p>Wire format (serde externally-tagged):
     * {@code {"Url": {...}}} | {@code {"Base64": {...}}} | {@code {"Binary": {...}}}.
     */
    public abstract static class VideoData {
        private VideoData() {}

        /** A URL pointing at the generated video. */
        public static class Url extends VideoData {
            private VideoUrlData value = new VideoUrlData();

            @JsonCreator
            Url() {}

            public Url(VideoUrlData value) { this.value = value; }

            public VideoUrlData getValue() { return value; }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Url)) return false;
                return Objects.equals(value, ((Url) o).value);
            }

            @Override
            public int hashCode() { return Objects.hash(value); }

            @Override
            public String toString() { return "VideoData.Url(" + value + ")"; }
        }

        /** Base64-encoded video. */
        public static class Base64 extends VideoData {
            private VideoBase64Data value = new VideoBase64Data();

            @JsonCreator
            Base64() {}

            public Base64(VideoBase64Data value) { this.value = value; }

            public VideoBase64Data getValue() { return value; }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Base64)) return false;
                return Objects.equals(value, ((Base64) o).value);
            }

            @Override
            public int hashCode() { return Objects.hash(value); }

            @Override
            public String toString() { return "VideoData.Base64(" + value + ")"; }
        }

        /** Raw binary video bytes (each element is a 0–255 byte value). */
        public static class Binary extends VideoData {
            private VideoBinaryData value = new VideoBinaryData();

            @JsonCreator
            Binary() {}

            public Binary(VideoBinaryData value) { this.value = value; }

            public VideoBinaryData getValue() { return value; }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Binary)) return false;
                return Objects.equals(value, ((Binary) o).value);
            }

            @Override
            public int hashCode() { return Objects.hash(value); }

            @Override
            public String toString() { return "VideoData.Binary(" + value + ")"; }
        }
    }

    /** Custom (de)serializer for {@link VideoData}: {@code {"Url": {...}}} | {@code {"Base64": {...}}} | {@code {"Binary": {...}}}. */
    public static class VideoDataSerializer extends JsonSerializer<VideoData> {
        @Override
        public void serialize(VideoData value, JsonGenerator gen, SerializerProvider serializers) throws IOException {
            gen.writeStartObject();
            if (value instanceof VideoData.Url) {
                gen.writeFieldName("Url");
                gen.writeTree(AimuxJson.MAPPER.valueToTree(((VideoData.Url) value).getValue()));
            } else if (value instanceof VideoData.Base64) {
                gen.writeFieldName("Base64");
                gen.writeTree(AimuxJson.MAPPER.valueToTree(((VideoData.Base64) value).getValue()));
            } else if (value instanceof VideoData.Binary) {
                gen.writeFieldName("Binary");
                gen.writeTree(AimuxJson.MAPPER.valueToTree(((VideoData.Binary) value).getValue()));
            } else {
                throw new IOException("Unknown VideoData: " + value);
            }
            gen.writeEndObject();
        }
    }

    /** Custom (de)serializer for {@link VideoData}. */
    public static class VideoDataDeserializer extends JsonDeserializer<VideoData> {
        @Override
        public VideoData deserialize(JsonParser p, DeserializationContext ctxt) throws IOException {
            JsonNode node = p.getCodec().readTree(p);
            if (!node.isObject() || node.size() != 1) {
                throw new IOException("VideoData must be a single-key externally-tagged object, got: " + node);
            }
            String tag = node.fieldNames().next();
            JsonNode inner = node.get(tag);
            if (!inner.isObject()) {
                inner = JsonNodeFactory.instance.objectNode();
            }
            switch (tag) {
                case "Url":
                    return new VideoData.Url(AimuxJson.MAPPER.treeToValue(inner, VideoUrlData.class));
                case "Base64":
                    return new VideoData.Base64(AimuxJson.MAPPER.treeToValue(inner, VideoBase64Data.class));
                case "Binary":
                    return new VideoData.Binary(AimuxJson.MAPPER.treeToValue(inner, VideoBinaryData.class));
                default:
                    throw new IOException("Unknown VideoData tag: '" + tag + "'");
            }
        }
    }

    /** Provider response metadata for video. */
    public static class VideoResponse {
        @JsonProperty("timestamp") private String timestamp;
        @JsonProperty("model_id") private String modelId;
        @JsonProperty("headers") private Map<String, String> headers;

        @JsonCreator
        VideoResponse() {}

        private VideoResponse(String timestamp, String modelId, Map<String, String> headers) {
            this.timestamp = timestamp;
            this.modelId = modelId;
            this.headers = headers;
        }

        public static VideoResponse of(String timestamp, String modelId, Map<String, String> headers) {
            return new VideoResponse(timestamp, modelId, headers);
        }

        public String getTimestamp() { return timestamp; }
        public String getModelId() { return modelId; }
        public Map<String, String> getHeaders() { return headers; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String timestamp;
            private String modelId;
            private Map<String, String> headers;

            public Builder timestamp(String v) { this.timestamp = v; return this; }
            public Builder modelId(String v) { this.modelId = v; return this; }
            public Builder headers(Map<String, String> v) { this.headers = v; return this; }

            public VideoResponse build() { return new VideoResponse(timestamp, modelId, headers); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof VideoResponse)) return false;
            VideoResponse that = (VideoResponse) o;
            return Objects.equals(timestamp, that.timestamp)
                && Objects.equals(modelId, that.modelId)
                && Objects.equals(headers, that.headers);
        }

        @Override
        public int hashCode() { return Objects.hash(timestamp, modelId, headers); }

        @Override
        public String toString() { return "VideoResponse(" + timestamp + ", " + modelId + ", " + headers + ")"; }
    }

    /** Result of a video generation call. */
    public static class VideoResult {
        @JsonProperty("videos") private List<VideoData> videos = new ArrayList<>();
        @JsonProperty("warnings") private List<JsonNode> warnings = new ArrayList<>();
        @JsonProperty("provider_metadata") private JsonNode providerMetadata;
        @JsonProperty("response") private VideoResponse response = new VideoResponse();

        @JsonCreator
        VideoResult() {}

        private VideoResult(List<VideoData> videos, List<JsonNode> warnings,
                            JsonNode providerMetadata, VideoResponse response) {
            this.videos = videos;
            this.warnings = warnings;
            this.providerMetadata = providerMetadata;
            this.response = response;
        }

        public static VideoResult of(List<VideoData> videos) {
            return new VideoResult(videos, new ArrayList<JsonNode>(), null, new VideoResponse());
        }

        public List<VideoData> getVideos() { return videos; }
        public List<JsonNode> getWarnings() { return warnings; }
        public JsonNode getProviderMetadata() { return providerMetadata; }
        public VideoResponse getResponse() { return response; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private List<VideoData> videos = new ArrayList<>();
            private List<JsonNode> warnings = new ArrayList<>();
            private JsonNode providerMetadata;
            private VideoResponse response = new VideoResponse();

            public Builder videos(List<VideoData> v) { this.videos = v; return this; }
            public Builder warnings(List<JsonNode> v) { this.warnings = v; return this; }
            public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }
            public Builder response(VideoResponse v) { this.response = v; return this; }

            public VideoResult build() {
                return new VideoResult(videos, warnings, providerMetadata, response);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof VideoResult)) return false;
            VideoResult that = (VideoResult) o;
            return Objects.equals(videos, that.videos)
                && Objects.equals(warnings, that.warnings)
                && Objects.equals(providerMetadata, that.providerMetadata)
                && Objects.equals(response, that.response);
        }

        @Override
        public int hashCode() { return Objects.hash(videos, warnings, providerMetadata, response); }

        @Override
        public String toString() { return "VideoResult(" + videos + ")"; }
    }

    /**
     * File payload for a {@link VideoFile}: a base64 string or raw binary bytes.
     *
     * <p>Wire format (serde externally-tagged): {@code {"Base64": "..."}} |
     * {@code {"Binary": [n,...]}}.
     */
    public abstract static class VideoFileData {
        private VideoFileData() {}

        /** Base64-encoded file data. */
        public static class Base64 extends VideoFileData {
            private String value = "";

            @JsonCreator
            Base64() {}

            public Base64(String value) { this.value = value; }

            public String getValue() { return value; }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Base64)) return false;
                return Objects.equals(value, ((Base64) o).value);
            }

            @Override
            public int hashCode() { return Objects.hash(value); }

            @Override
            public String toString() { return "VideoFileData.Base64(" + value + ")"; }
        }

        /** Raw binary file bytes (each element is a 0–255 byte value). */
        public static class Binary extends VideoFileData {
            private List<Integer> value = new ArrayList<>();

            @JsonCreator
            Binary() {}

            public Binary(List<Integer> value) { this.value = value; }

            public List<Integer> getValue() { return value; }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Binary)) return false;
                return Objects.equals(value, ((Binary) o).value);
            }

            @Override
            public int hashCode() { return Objects.hash(value); }

            @Override
            public String toString() { return "VideoFileData.Binary(" + value + ")"; }
        }
    }

    /** Custom (de)serializer for {@link VideoFileData}: {@code {"Base64": "..."}} | {@code {"Binary": [n,...]}}. */
    public static class VideoFileDataSerializer extends JsonSerializer<VideoFileData> {
        @Override
        public void serialize(VideoFileData value, JsonGenerator gen, SerializerProvider serializers) throws IOException {
            gen.writeStartObject();
            if (value instanceof VideoFileData.Base64) {
                gen.writeStringField("Base64", ((VideoFileData.Base64) value).getValue());
            } else if (value instanceof VideoFileData.Binary) {
                gen.writeArrayFieldStart("Binary");
                for (Integer b : ((VideoFileData.Binary) value).getValue()) {
                    gen.writeNumber(b);
                }
                gen.writeEndArray();
            } else {
                throw new IOException("Unknown VideoFileData: " + value);
            }
            gen.writeEndObject();
        }
    }

    /** Custom (de)serializer for {@link VideoFileData}. */
    public static class VideoFileDataDeserializer extends JsonDeserializer<VideoFileData> {
        @Override
        public VideoFileData deserialize(JsonParser p, DeserializationContext ctxt) throws IOException {
            JsonNode node = p.getCodec().readTree(p);
            if (!node.isObject() || node.size() != 1) {
                throw new IOException("VideoFileData must be a single-key externally-tagged object, got: " + node);
            }
            String tag = node.fieldNames().next();
            JsonNode inner = node.get(tag);
            switch (tag) {
                case "Base64":
                    return new VideoFileData.Base64(inner.asText());
                case "Binary": {
                    List<Integer> bytes = new ArrayList<>();
                    if (inner.isArray()) {
                        for (JsonNode item : inner) {
                            bytes.add(item.asInt());
                        }
                    }
                    return new VideoFileData.Binary(bytes);
                }
                default:
                    throw new IOException("Unknown VideoFileData tag: '" + tag + "'");
            }
        }
    }

    /** The {@code File} variant payload of {@link VideoFile}. */
    public static class VideoFileFileData {
        @JsonProperty("media_type") private String mediaType = "";
        @JsonProperty("data") private VideoFileData data = new VideoFileData.Base64("");

        @JsonCreator
        VideoFileFileData() {}

        private VideoFileFileData(String mediaType, VideoFileData data) {
            this.mediaType = mediaType;
            this.data = data;
        }

        public static VideoFileFileData of(String mediaType, VideoFileData data) {
            return new VideoFileFileData(mediaType, data);
        }

        public String getMediaType() { return mediaType; }
        public VideoFileData getData() { return data; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String mediaType = "";
            private VideoFileData data = new VideoFileData.Base64("");

            public Builder mediaType(String v) { this.mediaType = v; return this; }
            public Builder data(VideoFileData v) { this.data = v; return this; }

            public VideoFileFileData build() { return new VideoFileFileData(mediaType, data); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof VideoFileFileData)) return false;
            VideoFileFileData that = (VideoFileFileData) o;
            return Objects.equals(mediaType, that.mediaType)
                && Objects.equals(data, that.data);
        }

        @Override
        public int hashCode() { return Objects.hash(mediaType, data); }

        @Override
        public String toString() { return "VideoFileFileData(" + mediaType + ", " + data + ")"; }
    }

    /** The {@code Url} variant payload of {@link VideoFile}. */
    public static class VideoFileUrlData {
        @JsonProperty("url") private String url = "";
        @JsonProperty("media_type") private String mediaType;

        @JsonCreator
        VideoFileUrlData() {}

        private VideoFileUrlData(String url, String mediaType) {
            this.url = url;
            this.mediaType = mediaType;
        }

        public static VideoFileUrlData of(String url, String mediaType) {
            return new VideoFileUrlData(url, mediaType);
        }

        public String getUrl() { return url; }
        public String getMediaType() { return mediaType; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String url = "";
            private String mediaType;

            public Builder url(String v) { this.url = v; return this; }
            public Builder mediaType(String v) { this.mediaType = v; return this; }

            public VideoFileUrlData build() { return new VideoFileUrlData(url, mediaType); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof VideoFileUrlData)) return false;
            VideoFileUrlData that = (VideoFileUrlData) o;
            return Objects.equals(url, that.url)
                && Objects.equals(mediaType, that.mediaType);
        }

        @Override
        public int hashCode() { return Objects.hash(url, mediaType); }

        @Override
        public String toString() { return "VideoFileUrlData(" + url + ", " + mediaType + ")"; }
    }

    /**
     * A video or image file used for video editing or image-to-video generation.
     *
     * <p>Wire format (serde externally-tagged): {@code {"File": {...}}} |
     * {@code {"Url": {...}}}.
     */
    public abstract static class VideoFile {
        private VideoFile() {}

        /** Inline file data (base64 or raw bytes). */
        public static class File extends VideoFile {
            private VideoFileFileData value = new VideoFileFileData();

            @JsonCreator
            File() {}

            public File(VideoFileFileData value) { this.value = value; }

            public VideoFileFileData getValue() { return value; }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof File)) return false;
                return Objects.equals(value, ((File) o).value);
            }

            @Override
            public int hashCode() { return Objects.hash(value); }

            @Override
            public String toString() { return "VideoFile.File(" + value + ")"; }
        }

        /** A URL referencing the file. */
        public static class Url extends VideoFile {
            private VideoFileUrlData value = new VideoFileUrlData();

            @JsonCreator
            Url() {}

            public Url(VideoFileUrlData value) { this.value = value; }

            public VideoFileUrlData getValue() { return value; }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Url)) return false;
                return Objects.equals(value, ((Url) o).value);
            }

            @Override
            public int hashCode() { return Objects.hash(value); }

            @Override
            public String toString() { return "VideoFile.Url(" + value + ")"; }
        }
    }

    /** Custom (de)serializer for {@link VideoFile}: {@code {"File": {...}}} | {@code {"Url": {...}}}. */
    public static class VideoFileSerializer extends JsonSerializer<VideoFile> {
        @Override
        public void serialize(VideoFile value, JsonGenerator gen, SerializerProvider serializers) throws IOException {
            gen.writeStartObject();
            if (value instanceof VideoFile.File) {
                gen.writeFieldName("File");
                gen.writeTree(AimuxJson.MAPPER.valueToTree(((VideoFile.File) value).getValue()));
            } else if (value instanceof VideoFile.Url) {
                gen.writeFieldName("Url");
                gen.writeTree(AimuxJson.MAPPER.valueToTree(((VideoFile.Url) value).getValue()));
            } else {
                throw new IOException("Unknown VideoFile: " + value);
            }
            gen.writeEndObject();
        }
    }

    /** Custom (de)serializer for {@link VideoFile}. */
    public static class VideoFileDeserializer extends JsonDeserializer<VideoFile> {
        @Override
        public VideoFile deserialize(JsonParser p, DeserializationContext ctxt) throws IOException {
            JsonNode node = p.getCodec().readTree(p);
            if (!node.isObject() || node.size() != 1) {
                throw new IOException("VideoFile must be a single-key externally-tagged object, got: " + node);
            }
            String tag = node.fieldNames().next();
            JsonNode inner = node.get(tag);
            if (!inner.isObject()) {
                inner = JsonNodeFactory.instance.objectNode();
            }
            switch (tag) {
                case "File":
                    return new VideoFile.File(AimuxJson.MAPPER.treeToValue(inner, VideoFileFileData.class));
                case "Url":
                    return new VideoFile.Url(AimuxJson.MAPPER.treeToValue(inner, VideoFileUrlData.class));
                default:
                    throw new IOException("Unknown VideoFile tag: '" + tag + "'");
            }
        }
    }

    /** The role a frame image plays in video generation. */
    public enum VideoFrameType {
        @JsonProperty("FirstFrame") FIRST_FRAME,
        @JsonProperty("LastFrame") LAST_FRAME,
    }

    /** A role-tagged image input for image-to-video and first-last-frame generation. */
    public static class VideoFrameImage {
        @JsonProperty("image") private VideoFile image = new VideoFile.File(new VideoFileFileData());
        @JsonProperty("frame_type") private VideoFrameType frameType = VideoFrameType.FIRST_FRAME;

        @JsonCreator
        VideoFrameImage() {}

        private VideoFrameImage(VideoFile image, VideoFrameType frameType) {
            this.image = image;
            this.frameType = frameType;
        }

        public static VideoFrameImage of(VideoFile image, VideoFrameType frameType) {
            return new VideoFrameImage(image, frameType);
        }

        public VideoFile getImage() { return image; }
        public VideoFrameType getFrameType() { return frameType; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private VideoFile image = new VideoFile.File(new VideoFileFileData());
            private VideoFrameType frameType = VideoFrameType.FIRST_FRAME;

            public Builder image(VideoFile v) { this.image = v; return this; }
            public Builder frameType(VideoFrameType v) { this.frameType = v; return this; }

            public VideoFrameImage build() { return new VideoFrameImage(image, frameType); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof VideoFrameImage)) return false;
            VideoFrameImage that = (VideoFrameImage) o;
            return Objects.equals(image, that.image)
                && Objects.equals(frameType, that.frameType);
        }

        @Override
        public int hashCode() { return Objects.hash(image, frameType); }

        @Override
        public String toString() { return "VideoFrameImage(" + image + ", " + frameType + ")"; }
    }

    /** Options for video generation. */
    public static class VideoCallOptions {
        @JsonProperty("prompt") private String prompt;
        @JsonProperty("n") private Integer n;
        @JsonProperty("aspect_ratio") private String aspectRatio;
        @JsonProperty("resolution") private String resolution;
        @JsonProperty("duration") private Long duration;
        @JsonProperty("fps") private Double fps;
        @JsonProperty("seed") private Long seed;
        @JsonProperty("image") private VideoFile image;
        @JsonProperty("frame_images") private List<VideoFrameImage> frameImages;
        @JsonProperty("input_references") private List<VideoFile> inputReferences;
        @JsonProperty("generate_audio") private Boolean generateAudio;
        @JsonProperty("provider_options") private JsonNode providerOptions;
        @JsonProperty("headers") private Map<String, String> headers;

        @JsonCreator
        VideoCallOptions() {}

        private VideoCallOptions(String prompt, Integer n, String aspectRatio, String resolution,
                                 Long duration, Double fps, Long seed,
                                 VideoFile image, List<VideoFrameImage> frameImages,
                                 List<VideoFile> inputReferences, Boolean generateAudio,
                                 JsonNode providerOptions, Map<String, String> headers) {
            this.prompt = prompt;
            this.n = n;
            this.aspectRatio = aspectRatio;
            this.resolution = resolution;
            this.duration = duration;
            this.fps = fps;
            this.seed = seed;
            this.image = image;
            this.frameImages = frameImages;
            this.inputReferences = inputReferences;
            this.generateAudio = generateAudio;
            this.providerOptions = providerOptions;
            this.headers = headers;
        }

        public static VideoCallOptions of(String prompt) {
            return new VideoCallOptions(prompt, null, null, null, null, null, null,
                                        null, null, null, null, null, null);
        }

        public String getPrompt() { return prompt; }
        public Integer getN() { return n; }
        public String getAspectRatio() { return aspectRatio; }
        public String getResolution() { return resolution; }
        public Long getDuration() { return duration; }
        public Double getFps() { return fps; }
        public Long getSeed() { return seed; }
        public VideoFile getImage() { return image; }
        public List<VideoFrameImage> getFrameImages() { return frameImages; }
        public List<VideoFile> getInputReferences() { return inputReferences; }
        public Boolean getGenerateAudio() { return generateAudio; }
        public JsonNode getProviderOptions() { return providerOptions; }
        public Map<String, String> getHeaders() { return headers; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String prompt;
            private Integer n;
            private String aspectRatio;
            private String resolution;
            private Long duration;
            private Double fps;
            private Long seed;
            private VideoFile image;
            private List<VideoFrameImage> frameImages;
            private List<VideoFile> inputReferences;
            private Boolean generateAudio;
            private JsonNode providerOptions;
            private Map<String, String> headers;

            public Builder prompt(String v) { this.prompt = v; return this; }
            public Builder n(Integer v) { this.n = v; return this; }
            public Builder aspectRatio(String v) { this.aspectRatio = v; return this; }
            public Builder resolution(String v) { this.resolution = v; return this; }
            public Builder duration(Long v) { this.duration = v; return this; }
            public Builder fps(Double v) { this.fps = v; return this; }
            public Builder seed(Long v) { this.seed = v; return this; }
            public Builder image(VideoFile v) { this.image = v; return this; }
            public Builder frameImages(List<VideoFrameImage> v) { this.frameImages = v; return this; }
            public Builder inputReferences(List<VideoFile> v) { this.inputReferences = v; return this; }
            public Builder generateAudio(Boolean v) { this.generateAudio = v; return this; }
            public Builder providerOptions(JsonNode v) { this.providerOptions = v; return this; }
            public Builder headers(Map<String, String> v) { this.headers = v; return this; }

            public VideoCallOptions build() {
                return new VideoCallOptions(prompt, n, aspectRatio, resolution, duration, fps, seed,
                                            image, frameImages, inputReferences, generateAudio,
                                            providerOptions, headers);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof VideoCallOptions)) return false;
            VideoCallOptions that = (VideoCallOptions) o;
            return Objects.equals(prompt, that.prompt)
                && Objects.equals(n, that.n)
                && Objects.equals(aspectRatio, that.aspectRatio)
                && Objects.equals(resolution, that.resolution)
                && Objects.equals(duration, that.duration)
                && Objects.equals(fps, that.fps)
                && Objects.equals(seed, that.seed)
                && Objects.equals(image, that.image)
                && Objects.equals(frameImages, that.frameImages)
                && Objects.equals(inputReferences, that.inputReferences)
                && Objects.equals(generateAudio, that.generateAudio)
                && Objects.equals(providerOptions, that.providerOptions)
                && Objects.equals(headers, that.headers);
        }

        @Override
        public int hashCode() {
            return Objects.hash(prompt, n, aspectRatio, resolution, duration, fps, seed,
                                image, frameImages, inputReferences, generateAudio,
                                providerOptions, headers);
        }

        @Override
        public String toString() { return "VideoCallOptions(" + prompt + ")"; }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Search
    // ─────────────────────────────────────────────────────────────────────────────

    /** A single search result. */
    public static class SearchResultItem {
        @JsonProperty("title") private String title = "";
        @JsonProperty("url") private String url = "";
        @JsonProperty("content") private String content = "";
        @JsonProperty("raw_content") private String rawContent;
        @JsonProperty("score") private Double score;

        @JsonCreator
        SearchResultItem() {}

        private SearchResultItem(String title, String url, String content,
                                 String rawContent, Double score) {
            this.title = title;
            this.url = url;
            this.content = content;
            this.rawContent = rawContent;
            this.score = score;
        }

        public static SearchResultItem of(String title, String url, String content) {
            return new SearchResultItem(title, url, content, null, null);
        }

        public String getTitle() { return title; }
        public String getUrl() { return url; }
        public String getContent() { return content; }
        public String getRawContent() { return rawContent; }
        public Double getScore() { return score; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String title = "";
            private String url = "";
            private String content = "";
            private String rawContent;
            private Double score;

            public Builder title(String v) { this.title = v; return this; }
            public Builder url(String v) { this.url = v; return this; }
            public Builder content(String v) { this.content = v; return this; }
            public Builder rawContent(String v) { this.rawContent = v; return this; }
            public Builder score(Double v) { this.score = v; return this; }

            public SearchResultItem build() {
                return new SearchResultItem(title, url, content, rawContent, score);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof SearchResultItem)) return false;
            SearchResultItem that = (SearchResultItem) o;
            return Objects.equals(title, that.title)
                && Objects.equals(url, that.url)
                && Objects.equals(content, that.content)
                && Objects.equals(rawContent, that.rawContent)
                && Objects.equals(score, that.score);
        }

        @Override
        public int hashCode() { return Objects.hash(title, url, content, rawContent, score); }

        @Override
        public String toString() { return "SearchResultItem(" + title + ")"; }
    }

    /** Provider response metadata for search. */
    public static class SearchResponse {
        @JsonProperty("headers") private Map<String, String> headers;
        @JsonProperty("body") private JsonNode body;

        @JsonCreator
        SearchResponse() {}

        private SearchResponse(Map<String, String> headers, JsonNode body) {
            this.headers = headers;
            this.body = body;
        }

        public static SearchResponse of(Map<String, String> headers, JsonNode body) {
            return new SearchResponse(headers, body);
        }

        public Map<String, String> getHeaders() { return headers; }
        public JsonNode getBody() { return body; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private Map<String, String> headers;
            private JsonNode body;

            public Builder headers(Map<String, String> v) { this.headers = v; return this; }
            public Builder body(JsonNode v) { this.body = v; return this; }

            public SearchResponse build() { return new SearchResponse(headers, body); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof SearchResponse)) return false;
            SearchResponse that = (SearchResponse) o;
            return Objects.equals(headers, that.headers)
                && Objects.equals(body, that.body);
        }

        @Override
        public int hashCode() { return Objects.hash(headers, body); }

        @Override
        public String toString() { return "SearchResponse(" + headers + ", " + body + ")"; }
    }

    /** Result of a search call. */
    public static class SearchResult {
        @JsonProperty("results") private List<SearchResultItem> results = new ArrayList<>();
        @JsonProperty("answer") private String answer;
        @JsonProperty("provider_metadata") private JsonNode providerMetadata;
        @JsonProperty("warnings") private List<JsonNode> warnings = new ArrayList<>();
        @JsonProperty("response") private SearchResponse response;

        @JsonCreator
        SearchResult() {}

        private SearchResult(List<SearchResultItem> results, String answer, JsonNode providerMetadata,
                             List<JsonNode> warnings, SearchResponse response) {
            this.results = results;
            this.answer = answer;
            this.providerMetadata = providerMetadata;
            this.warnings = warnings;
            this.response = response;
        }

        public static SearchResult of(List<SearchResultItem> results) {
            return new SearchResult(results, null, null, new ArrayList<JsonNode>(), null);
        }

        public List<SearchResultItem> getResults() { return results; }
        public String getAnswer() { return answer; }
        public JsonNode getProviderMetadata() { return providerMetadata; }
        public List<JsonNode> getWarnings() { return warnings; }
        public SearchResponse getResponse() { return response; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private List<SearchResultItem> results = new ArrayList<>();
            private String answer;
            private JsonNode providerMetadata;
            private List<JsonNode> warnings = new ArrayList<>();
            private SearchResponse response;

            public Builder results(List<SearchResultItem> v) { this.results = v; return this; }
            public Builder answer(String v) { this.answer = v; return this; }
            public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }
            public Builder warnings(List<JsonNode> v) { this.warnings = v; return this; }
            public Builder response(SearchResponse v) { this.response = v; return this; }

            public SearchResult build() {
                return new SearchResult(results, answer, providerMetadata, warnings, response);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof SearchResult)) return false;
            SearchResult that = (SearchResult) o;
            return Objects.equals(results, that.results)
                && Objects.equals(answer, that.answer)
                && Objects.equals(providerMetadata, that.providerMetadata)
                && Objects.equals(warnings, that.warnings)
                && Objects.equals(response, that.response);
        }

        @Override
        public int hashCode() { return Objects.hash(results, answer, providerMetadata, warnings, response); }

        @Override
        public String toString() { return "SearchResult(" + results + ")"; }
    }

    /** Options for a search call. */
    public static class SearchCallOptions {
        @JsonProperty("query") private String query = "";
        @JsonProperty("max_results") private Integer maxResults;
        @JsonProperty("include_raw_content") private Boolean includeRawContent;
        @JsonProperty("time_range") private String timeRange;
        @JsonProperty("include_domains") private List<String> includeDomains = new ArrayList<>();
        @JsonProperty("exclude_domains") private List<String> excludeDomains = new ArrayList<>();
        @JsonProperty("provider_options") private JsonNode providerOptions;
        @JsonProperty("headers") private Map<String, String> headers;

        @JsonCreator
        SearchCallOptions() {}

        private SearchCallOptions(String query, Integer maxResults, Boolean includeRawContent,
                                  String timeRange, List<String> includeDomains,
                                  List<String> excludeDomains, JsonNode providerOptions,
                                  Map<String, String> headers) {
            this.query = query;
            this.maxResults = maxResults;
            this.includeRawContent = includeRawContent;
            this.timeRange = timeRange;
            this.includeDomains = includeDomains;
            this.excludeDomains = excludeDomains;
            this.providerOptions = providerOptions;
            this.headers = headers;
        }

        public static SearchCallOptions of(String query) {
            return new SearchCallOptions(query, null, null, null, new ArrayList<String>(),
                new ArrayList<String>(), null, null);
        }

        public String getQuery() { return query; }
        public Integer getMaxResults() { return maxResults; }
        public Boolean getIncludeRawContent() { return includeRawContent; }
        public String getTimeRange() { return timeRange; }
        public List<String> getIncludeDomains() { return includeDomains; }
        public List<String> getExcludeDomains() { return excludeDomains; }
        public JsonNode getProviderOptions() { return providerOptions; }
        public Map<String, String> getHeaders() { return headers; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String query = "";
            private Integer maxResults;
            private Boolean includeRawContent;
            private String timeRange;
            private List<String> includeDomains = new ArrayList<>();
            private List<String> excludeDomains = new ArrayList<>();
            private JsonNode providerOptions;
            private Map<String, String> headers;

            public Builder query(String v) { this.query = v; return this; }
            public Builder maxResults(Integer v) { this.maxResults = v; return this; }
            public Builder includeRawContent(Boolean v) { this.includeRawContent = v; return this; }
            public Builder timeRange(String v) { this.timeRange = v; return this; }
            public Builder includeDomains(List<String> v) { this.includeDomains = v; return this; }
            public Builder excludeDomains(List<String> v) { this.excludeDomains = v; return this; }
            public Builder providerOptions(JsonNode v) { this.providerOptions = v; return this; }
            public Builder headers(Map<String, String> v) { this.headers = v; return this; }

            public SearchCallOptions build() {
                return new SearchCallOptions(query, maxResults, includeRawContent, timeRange,
                    includeDomains, excludeDomains, providerOptions, headers);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof SearchCallOptions)) return false;
            SearchCallOptions that = (SearchCallOptions) o;
            return Objects.equals(query, that.query)
                && Objects.equals(maxResults, that.maxResults)
                && Objects.equals(includeRawContent, that.includeRawContent)
                && Objects.equals(timeRange, that.timeRange)
                && Objects.equals(includeDomains, that.includeDomains)
                && Objects.equals(excludeDomains, that.excludeDomains)
                && Objects.equals(providerOptions, that.providerOptions)
                && Objects.equals(headers, that.headers);
        }

        @Override
        public int hashCode() {
            return Objects.hash(query, maxResults, includeRawContent, timeRange,
                includeDomains, excludeDomains, providerOptions, headers);
        }

        @Override
        public String toString() { return "SearchCallOptions(" + query + ")"; }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Files (upload)
    // ─────────────────────────────────────────────────────────────────────────────

    /** Result of a file upload. */
    public static class UploadFileResult {
        @JsonProperty("provider_reference") private Map<String, String> providerReference = new HashMap<>();
        @JsonProperty("media_type") private String mediaType;
        @JsonProperty("filename") private String filename;
        @JsonProperty("provider_metadata") private JsonNode providerMetadata;
        @JsonProperty("warnings") private List<JsonNode> warnings = new ArrayList<>();

        @JsonCreator
        UploadFileResult() {}

        private UploadFileResult(Map<String, String> providerReference, String mediaType,
                                 String filename, JsonNode providerMetadata, List<JsonNode> warnings) {
            this.providerReference = providerReference;
            this.mediaType = mediaType;
            this.filename = filename;
            this.providerMetadata = providerMetadata;
            this.warnings = warnings;
        }

        public static UploadFileResult of(Map<String, String> providerReference) {
            return new UploadFileResult(providerReference, null, null, null, new ArrayList<JsonNode>());
        }

        public Map<String, String> getProviderReference() { return providerReference; }
        public String getMediaType() { return mediaType; }
        public String getFilename() { return filename; }
        public JsonNode getProviderMetadata() { return providerMetadata; }
        public List<JsonNode> getWarnings() { return warnings; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private Map<String, String> providerReference = new HashMap<>();
            private String mediaType;
            private String filename;
            private JsonNode providerMetadata;
            private List<JsonNode> warnings = new ArrayList<>();

            public Builder providerReference(Map<String, String> v) { this.providerReference = v; return this; }
            public Builder mediaType(String v) { this.mediaType = v; return this; }
            public Builder filename(String v) { this.filename = v; return this; }
            public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }
            public Builder warnings(List<JsonNode> v) { this.warnings = v; return this; }

            public UploadFileResult build() {
                return new UploadFileResult(providerReference, mediaType, filename, providerMetadata, warnings);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof UploadFileResult)) return false;
            UploadFileResult that = (UploadFileResult) o;
            return Objects.equals(providerReference, that.providerReference)
                && Objects.equals(mediaType, that.mediaType)
                && Objects.equals(filename, that.filename)
                && Objects.equals(providerMetadata, that.providerMetadata)
                && Objects.equals(warnings, that.warnings);
        }

        @Override
        public int hashCode() {
            return Objects.hash(providerReference, mediaType, filename, providerMetadata, warnings);
        }

        @Override
        public String toString() { return "UploadFileResult(" + providerReference + ")"; }
    }

    /** Options for a file upload. */
    public static class UploadFileCallOptions {
        @JsonProperty("data") private JsonNode data = JsonNodeFactory.instance.objectNode();
        @JsonProperty("media_type") private String mediaType = "";
        @JsonProperty("filename") private String filename;
        @JsonProperty("provider_options") private JsonNode providerOptions;
        @JsonProperty("headers") private Map<String, String> headers;

        @JsonCreator
        UploadFileCallOptions() {}

        private UploadFileCallOptions(JsonNode data, String mediaType, String filename,
                                      JsonNode providerOptions, Map<String, String> headers) {
            this.data = data;
            this.mediaType = mediaType;
            this.filename = filename;
            this.providerOptions = providerOptions;
            this.headers = headers;
        }

        public static UploadFileCallOptions of(JsonNode data, String mediaType) {
            return new UploadFileCallOptions(data, mediaType, null, null, null);
        }

        public JsonNode getData() { return data; }
        public String getMediaType() { return mediaType; }
        public String getFilename() { return filename; }
        public JsonNode getProviderOptions() { return providerOptions; }
        public Map<String, String> getHeaders() { return headers; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private JsonNode data = JsonNodeFactory.instance.objectNode();
            private String mediaType = "";
            private String filename;
            private JsonNode providerOptions;
            private Map<String, String> headers;

            public Builder data(JsonNode v) { this.data = v; return this; }
            public Builder mediaType(String v) { this.mediaType = v; return this; }
            public Builder filename(String v) { this.filename = v; return this; }
            public Builder providerOptions(JsonNode v) { this.providerOptions = v; return this; }
            public Builder headers(Map<String, String> v) { this.headers = v; return this; }

            public UploadFileCallOptions build() {
                return new UploadFileCallOptions(data, mediaType, filename, providerOptions, headers);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof UploadFileCallOptions)) return false;
            UploadFileCallOptions that = (UploadFileCallOptions) o;
            return Objects.equals(data, that.data)
                && Objects.equals(mediaType, that.mediaType)
                && Objects.equals(filename, that.filename)
                && Objects.equals(providerOptions, that.providerOptions)
                && Objects.equals(headers, that.headers);
        }

        @Override
        public int hashCode() { return Objects.hash(data, mediaType, filename, providerOptions, headers); }

        @Override
        public String toString() { return "UploadFileCallOptions(" + mediaType + ")"; }
    }
}













