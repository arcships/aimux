package ai.arcships.aimux;

import com.fasterxml.jackson.annotation.JsonAutoDetect;
import com.fasterxml.jackson.annotation.JsonCreator;
import com.fasterxml.jackson.annotation.JsonInclude;
import com.fasterxml.jackson.annotation.JsonProperty;
import com.fasterxml.jackson.annotation.JsonSubTypes;
import com.fasterxml.jackson.annotation.JsonTypeInfo;
import com.fasterxml.jackson.annotation.JsonTypeName;
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
import com.fasterxml.jackson.databind.node.TextNode;

import java.io.IOException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.function.Supplier;

/**
 * aimux — typed data classes mirroring the Kotlin binding in
 * `bindings/kotlin` (which in turn mirrors the ts-rs output in
 * `bindings/node/src/types`).
 *
 * Field names are camelCase in Java and mapped to the wire format's snake_case
 * via {@link JsonProperty}. The raw JSON boundary is handled by
 * {@link TypedModel} — callers of this layer never parse JSON by hand.
 *
 * These types are intentionally lenient on decode (unknown keys ignored, every
 * field has a default) so that future provider/engine additions do not break
 * existing clients. The serialization config lives in {@link Types.AimuxJson}.
 */
public final class Types {
    private Types() {}

    // ─────────────────────────────────────────────────────────────────────────────
    // Shared ObjectMapper.
    //
    //  - FAIL_ON_UNKNOWN_PROPERTIES disabled : tolerate forward-compatible fields.
    //  - NON_NULL inclusion                   : omit null fields on encode so
    //                                           GenerateTextOptions only carries
    //                                           fields the caller actually set.
    //  - FIELD visibility ANY                 : read/write fields directly; the
    //                                           package-private @JsonCreator ctor
    //                                           handles instantiation.
    // ─────────────────────────────────────────────────────────────────────────────

    /** Shared JSON mapper for the typed layer. */
    public static final class AimuxJson {
        private AimuxJson() {}

        public static final ObjectMapper MAPPER = createMapper();

        /**
         * Inner mapper used by the externally-tagged polymorphic serializers
         * ({@link ContentPartSerializer}, {@link StreamPartSerializer},
         * {@link GenerateContentSerializer}) to serialize the concrete variant
         * as a plain POJO. It deliberately does NOT register those four
         * polymorphic serializers, so {@code valueToTree} on a concrete subtype
         * (e.g. {@code StreamPart.TextDelta}) uses default field serialization
         * instead of re-entering the polymorphic serializer — which would
         * otherwise infinite-recurse ({@code StackOverflowError}). Non-recursing
         * serializers that concrete variants may transitively contain
         * ({@link ToolChoice}, {@link FileBytes}, {@link FileData}) are kept.
         */
        public static final ObjectMapper INNER_MAPPER = createInnerMapper();

        private static ObjectMapper createMapper() {
            return configure(new ObjectMapper(), true);
        }

        private static ObjectMapper createInnerMapper() {
            return configure(new ObjectMapper(), false);
        }

        private static ObjectMapper configure(ObjectMapper m, boolean polymorphic) {
            m.configure(DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES, false);
            m.setSerializationInclusion(JsonInclude.Include.NON_NULL);
            m.setVisibility(PropertyAccessor.FIELD, JsonAutoDetect.Visibility.ANY);
            m.setVisibility(PropertyAccessor.GETTER, JsonAutoDetect.Visibility.NONE);
            m.setVisibility(PropertyAccessor.SETTER, JsonAutoDetect.Visibility.NONE);
            m.setVisibility(PropertyAccessor.CREATOR, JsonAutoDetect.Visibility.ANY);

            SimpleModule module = new SimpleModule("aimux");
            // Non-recursing serializers — safe on both mappers (concrete variants
            // may transitively contain these types).
            module.addSerializer(ToolChoice.class, new ToolChoiceSerializer());
            module.addDeserializer(ToolChoice.class, new ToolChoiceDeserializer());
            module.addSerializer(FileBytes.class, new FileBytesSerializer());
            module.addDeserializer(FileBytes.class, new FileBytesDeserializer());
            module.addSerializer(FileData.class, new FileDataSerializer());
            module.addDeserializer(FileData.class, new FileDataDeserializer());
            // Externally-tagged polymorphic serializers — only on the public
            // mapper. They use INNER_MAPPER internally to avoid self-recursion.
            if (polymorphic) {
                module.addSerializer(MessageContent.class, new MessageContentSerializer());
                module.addDeserializer(MessageContent.class, new MessageContentDeserializer());
                module.addSerializer(ContentPart.class, new ContentPartSerializer());
                module.addDeserializer(ContentPart.class, new ContentPartDeserializer());
                module.addSerializer(GenerateContent.class, new GenerateContentSerializer());
                module.addDeserializer(GenerateContent.class, new GenerateContentDeserializer());
                module.addSerializer(StreamPart.class, new StreamPartSerializer());
                module.addDeserializer(StreamPart.class, new StreamPartDeserializer());
            }
            m.registerModule(module);
            return m;
        }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Enums (simple string enums on the wire).
    // ─────────────────────────────────────────────────────────────────────────────

    public enum Role {
        @JsonProperty("system") SYSTEM,
        @JsonProperty("user") USER,
        @JsonProperty("assistant") ASSISTANT,
        @JsonProperty("tool") TOOL,
    }

    public enum FinishReasonUnified {
        @JsonProperty("stop") STOP,
        @JsonProperty("length") LENGTH,
        @JsonProperty("content-filter") CONTENT_FILTER,
        @JsonProperty("tool-calls") TOOL_CALLS,
        @JsonProperty("error") ERROR,
        @JsonProperty("other") OTHER,
    }

    public enum ReasoningEffort {
        @JsonProperty("provider-default") PROVIDER_DEFAULT,
        @JsonProperty("none") NONE,
        @JsonProperty("minimal") MINIMAL,
        @JsonProperty("low") LOW,
        @JsonProperty("medium") MEDIUM,
        @JsonProperty("high") HIGH,
        @JsonProperty("xhigh") XHIGH,
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Core nested types.
    // ─────────────────────────────────────────────────────────────────────────────

    /**
     * Token usage detail (with cache breakdown). Every field is nullable because
     * providers do not always populate all breakdowns.
     */
    public static class TokenUsage {
        @JsonProperty("total") private Long total;
        @JsonProperty("no_cache") private Long noCache;
        @JsonProperty("cache_read") private Long cacheRead;
        @JsonProperty("cache_write") private Long cacheWrite;
        @JsonProperty("text") private Long text;
        @JsonProperty("reasoning") private Long reasoning;

        @JsonCreator
        TokenUsage() {}

        private TokenUsage(Long total, Long noCache, Long cacheRead, Long cacheWrite, Long text, Long reasoning) {
            this.total = total;
            this.noCache = noCache;
            this.cacheRead = cacheRead;
            this.cacheWrite = cacheWrite;
            this.text = text;
            this.reasoning = reasoning;
        }

        /** Convenience: a usage with only the total token count set. */
        public static TokenUsage of(Long total) {
            return new TokenUsage(total, null, null, null, null, null);
        }

        public Long getTotal() { return total; }
        public Long getNoCache() { return noCache; }
        public Long getCacheRead() { return cacheRead; }
        public Long getCacheWrite() { return cacheWrite; }
        public Long getText() { return text; }
        public Long getReasoning() { return reasoning; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private Long total;
            private Long noCache;
            private Long cacheRead;
            private Long cacheWrite;
            private Long text;
            private Long reasoning;

            public Builder total(Long v) { this.total = v; return this; }
            public Builder noCache(Long v) { this.noCache = v; return this; }
            public Builder cacheRead(Long v) { this.cacheRead = v; return this; }
            public Builder cacheWrite(Long v) { this.cacheWrite = v; return this; }
            public Builder text(Long v) { this.text = v; return this; }
            public Builder reasoning(Long v) { this.reasoning = v; return this; }

            public TokenUsage build() {
                return new TokenUsage(total, noCache, cacheRead, cacheWrite, text, reasoning);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof TokenUsage)) return false;
            TokenUsage that = (TokenUsage) o;
            return Objects.equals(total, that.total)
                && Objects.equals(noCache, that.noCache)
                && Objects.equals(cacheRead, that.cacheRead)
                && Objects.equals(cacheWrite, that.cacheWrite)
                && Objects.equals(text, that.text)
                && Objects.equals(reasoning, that.reasoning);
        }

        @Override
        public int hashCode() {
            return Objects.hash(total, noCache, cacheRead, cacheWrite, text, reasoning);
        }
    }

    /**
     * Token usage statistics.
     *
     * Mirrors `Usage.ts`: `{ input_tokens: TokenUsage, output_tokens: TokenUsage,
     * raw?: JsonValue | null }`.
     */
    public static class Usage {
        @JsonProperty("input_tokens") private TokenUsage inputTokens = new TokenUsage();
        @JsonProperty("output_tokens") private TokenUsage outputTokens = new TokenUsage();
        @JsonProperty("raw") private JsonNode raw;

        @JsonCreator
        Usage() {}

        private Usage(TokenUsage inputTokens, TokenUsage outputTokens, JsonNode raw) {
            this.inputTokens = inputTokens;
            this.outputTokens = outputTokens;
            this.raw = raw;
        }

        /** Convenience for tests/inspection. */
        public static Usage of(Long input, Long output) {
            return new Usage(TokenUsage.of(input), TokenUsage.of(output), null);
        }

        public TokenUsage getInputTokens() { return inputTokens; }
        public TokenUsage getOutputTokens() { return outputTokens; }
        public JsonNode getRaw() { return raw; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private TokenUsage inputTokens = new TokenUsage();
            private TokenUsage outputTokens = new TokenUsage();
            private JsonNode raw;

            public Builder inputTokens(TokenUsage v) { this.inputTokens = v; return this; }
            public Builder outputTokens(TokenUsage v) { this.outputTokens = v; return this; }
            public Builder raw(JsonNode v) { this.raw = v; return this; }

            public Usage build() { return new Usage(inputTokens, outputTokens, raw); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof Usage)) return false;
            Usage that = (Usage) o;
            return Objects.equals(inputTokens, that.inputTokens)
                && Objects.equals(outputTokens, that.outputTokens)
                && Objects.equals(raw, that.raw);
        }

        @Override
        public int hashCode() {
            return Objects.hash(inputTokens, outputTokens, raw);
        }
    }

    /**
     * Unified finish reason.
     *
     * Mirrors `FinishReason.ts`: `{ unified: FinishReasonUnified, raw: string | null }`.
     */
    public static class FinishReason {
        @JsonProperty("unified") private FinishReasonUnified unified = FinishReasonUnified.OTHER;
        @JsonProperty("raw") private String raw;

        @JsonCreator
        FinishReason() {}

        private FinishReason(FinishReasonUnified unified, String raw) {
            this.unified = unified;
            this.raw = raw;
        }

        public FinishReasonUnified getUnified() { return unified; }
        public String getRaw() { return raw; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private FinishReasonUnified unified = FinishReasonUnified.OTHER;
            private String raw;

            public Builder unified(FinishReasonUnified v) { this.unified = v; return this; }
            public Builder raw(String v) { this.raw = v; return this; }

            public FinishReason build() { return new FinishReason(unified, raw); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof FinishReason)) return false;
            FinishReason that = (FinishReason) o;
            return unified == that.unified && Objects.equals(raw, that.raw);
        }

        @Override
        public int hashCode() {
            return Objects.hash(unified, raw);
        }
    }

    /**
     * Metadata about the API response.
     *
     * Mirrors `ResponseMetadata.ts`: `{ id: string | null, timestamp: string | null,
     * model_id: string | null }`.
     */
    public static class ResponseMetadata {
        @JsonProperty("id") private String id;
        @JsonProperty("timestamp") private String timestamp;
        @JsonProperty("model_id") private String modelId;

        @JsonCreator
        ResponseMetadata() {}

        private ResponseMetadata(String id, String timestamp, String modelId) {
            this.id = id;
            this.timestamp = timestamp;
            this.modelId = modelId;
        }

        public String getId() { return id; }
        public String getTimestamp() { return timestamp; }
        public String getModelId() { return modelId; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String id;
            private String timestamp;
            private String modelId;

            public Builder id(String v) { this.id = v; return this; }
            public Builder timestamp(String v) { this.timestamp = v; return this; }
            public Builder modelId(String v) { this.modelId = v; return this; }

            public ResponseMetadata build() { return new ResponseMetadata(id, timestamp, modelId); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof ResponseMetadata)) return false;
            ResponseMetadata that = (ResponseMetadata) o;
            return Objects.equals(id, that.id)
                && Objects.equals(timestamp, that.timestamp)
                && Objects.equals(modelId, that.modelId);
        }

        @Override
        public int hashCode() {
            return Objects.hash(id, timestamp, modelId);
        }
    }

    /**
     * A tool call requested by the model.
     *
     * Mirrors `ToolCall.ts`: `{ tool_call_id, tool_name, input: JsonValue,
     * provider_executed?: bool | null, dynamic?: bool | null }`.
     *
     * `input` is a {@link JsonNode} because it is usually an arbitrary JSON object
     * (the tool arguments) whose shape is tool-specific.
     */
    public static class ToolCall {
        @JsonProperty("tool_call_id") private String toolCallId = "";
        @JsonProperty("tool_name") private String toolName = "";
        @JsonProperty("input") private JsonNode input = emptyObject();
        @JsonProperty("provider_executed") private Boolean providerExecuted;
        @JsonProperty("dynamic") private Boolean dynamic;

        @JsonCreator
        ToolCall() {}

        private ToolCall(String toolCallId, String toolName, JsonNode input, Boolean providerExecuted, Boolean dynamic) {
            this.toolCallId = toolCallId;
            this.toolName = toolName;
            this.input = input;
            this.providerExecuted = providerExecuted;
            this.dynamic = dynamic;
        }

        public String getToolCallId() { return toolCallId; }
        public String getToolName() { return toolName; }
        public JsonNode getInput() { return input; }
        public Boolean getProviderExecuted() { return providerExecuted; }
        public Boolean getDynamic() { return dynamic; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String toolCallId = "";
            private String toolName = "";
            private JsonNode input = emptyObject();
            private Boolean providerExecuted;
            private Boolean dynamic;

            public Builder toolCallId(String v) { this.toolCallId = v; return this; }
            public Builder toolName(String v) { this.toolName = v; return this; }
            public Builder input(JsonNode v) { this.input = v; return this; }
            public Builder providerExecuted(Boolean v) { this.providerExecuted = v; return this; }
            public Builder dynamic(Boolean v) { this.dynamic = v; return this; }

            public ToolCall build() { return new ToolCall(toolCallId, toolName, input, providerExecuted, dynamic); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof ToolCall)) return false;
            ToolCall that = (ToolCall) o;
            return Objects.equals(toolCallId, that.toolCallId)
                && Objects.equals(toolName, that.toolName)
                && Objects.equals(input, that.input)
                && Objects.equals(providerExecuted, that.providerExecuted)
                && Objects.equals(dynamic, that.dynamic);
        }

        @Override
        public int hashCode() {
            return Objects.hash(toolCallId, toolName, input, providerExecuted, dynamic);
        }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Tools (input side of GenerateTextOptions).
    //
    // `Tool` is an internally-tagged union on `type` (`"function" | "provider"`).
    // ─────────────────────────────────────────────────────────────────────────────

    /**
     * A user-defined function tool.
     *
     * Mirrors `FunctionTool.ts`. `input_schema` is a {@link JsonNode} (a JSON Schema).
     */
    public static class FunctionTool {
        @JsonProperty("name") private String name = "";
        @JsonProperty("description") private String description;
        @JsonProperty("input_schema") private JsonNode inputSchema = emptyObject();
        @JsonProperty("strict") private Boolean strict;
        @JsonProperty("provider_options") private Map<String, JsonNode> providerOptions;
        @JsonProperty("input_examples") private List<JsonNode> inputExamples;

        @JsonCreator
        FunctionTool() {}

        private FunctionTool(String name, String description, JsonNode inputSchema, Boolean strict,
                             Map<String, JsonNode> providerOptions, List<JsonNode> inputExamples) {
            this.name = name;
            this.description = description;
            this.inputSchema = inputSchema;
            this.strict = strict;
            this.providerOptions = providerOptions;
            this.inputExamples = inputExamples;
        }

        public String getName() { return name; }
        public String getDescription() { return description; }
        public JsonNode getInputSchema() { return inputSchema; }
        public Boolean getStrict() { return strict; }
        public Map<String, JsonNode> getProviderOptions() { return providerOptions; }
        public List<JsonNode> getInputExamples() { return inputExamples; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String name = "";
            private String description;
            private JsonNode inputSchema = emptyObject();
            private Boolean strict;
            private Map<String, JsonNode> providerOptions;
            private List<JsonNode> inputExamples;

            public Builder name(String v) { this.name = v; return this; }
            public Builder description(String v) { this.description = v; return this; }
            public Builder inputSchema(JsonNode v) { this.inputSchema = v; return this; }
            public Builder strict(Boolean v) { this.strict = v; return this; }
            public Builder providerOptions(Map<String, JsonNode> v) { this.providerOptions = v; return this; }
            public Builder inputExamples(List<JsonNode> v) { this.inputExamples = v; return this; }

            public FunctionTool build() {
                return new FunctionTool(name, description, inputSchema, strict, providerOptions, inputExamples);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof FunctionTool)) return false;
            FunctionTool that = (FunctionTool) o;
            return Objects.equals(name, that.name)
                && Objects.equals(description, that.description)
                && Objects.equals(inputSchema, that.inputSchema)
                && Objects.equals(strict, that.strict)
                && Objects.equals(providerOptions, that.providerOptions)
                && Objects.equals(inputExamples, that.inputExamples);
        }

        @Override
        public int hashCode() {
            return Objects.hash(name, description, inputSchema, strict, providerOptions, inputExamples);
        }
    }

    /**
     * A provider-defined tool (e.g. `anthropic.web_search_20250305`).
     *
     * Mirrors `ProviderTool.ts`.
     */
    public static class ProviderTool {
        @JsonProperty("id") private String id = "";
        @JsonProperty("name") private String name = "";
        @JsonProperty("args") private JsonNode args = emptyObject();

        @JsonCreator
        ProviderTool() {}

        private ProviderTool(String id, String name, JsonNode args) {
            this.id = id;
            this.name = name;
            this.args = args;
        }

        public String getId() { return id; }
        public String getName() { return name; }
        public JsonNode getArgs() { return args; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String id = "";
            private String name = "";
            private JsonNode args = emptyObject();

            public Builder id(String v) { this.id = v; return this; }
            public Builder name(String v) { this.name = v; return this; }
            public Builder args(JsonNode v) { this.args = v; return this; }

            public ProviderTool build() { return new ProviderTool(id, name, args); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof ProviderTool)) return false;
            ProviderTool that = (ProviderTool) o;
            return Objects.equals(id, that.id)
                && Objects.equals(name, that.name)
                && Objects.equals(args, that.args);
        }

        @Override
        public int hashCode() {
            return Objects.hash(id, name, args);
        }
    }

    /**
     * A tool that can be either a function tool or a provider tool.
     *
     * Mirrors `Tool.ts`. Serialized as `{"type":"function", ...}` /
     * `{"type":"provider", ...}` (internal `"type"` discriminator).
     */
    @JsonTypeInfo(use = JsonTypeInfo.Id.NAME, include = JsonTypeInfo.As.PROPERTY, property = "type")
    @JsonSubTypes({
        @JsonSubTypes.Type(value = Tool.Function.class, name = "function"),
        @JsonSubTypes.Type(value = Tool.Provider.class, name = "provider"),
    })
    public abstract static class Tool {
        private Tool() {}

        @JsonTypeName("function")
        public static class Function extends Tool {
            @JsonProperty("name") private String name = "";
            @JsonProperty("description") private String description;
            @JsonProperty("input_schema") private JsonNode inputSchema = emptyObject();
            @JsonProperty("strict") private Boolean strict;
            @JsonProperty("provider_options") private Map<String, JsonNode> providerOptions;
            @JsonProperty("input_examples") private List<JsonNode> inputExamples;

            @JsonCreator
            Function() {}

            private Function(String name, String description, JsonNode inputSchema, Boolean strict,
                             Map<String, JsonNode> providerOptions, List<JsonNode> inputExamples) {
                this.name = name;
                this.description = description;
                this.inputSchema = inputSchema;
                this.strict = strict;
                this.providerOptions = providerOptions;
                this.inputExamples = inputExamples;
            }

            /** Convenience constructor from a {@link FunctionTool}. */
            public static Function from(FunctionTool tool) {
                return new Function(tool.name, tool.description, tool.inputSchema, tool.strict,
                    tool.providerOptions, tool.inputExamples);
            }

            public String getName() { return name; }
            public String getDescription() { return description; }
            public JsonNode getInputSchema() { return inputSchema; }
            public Boolean getStrict() { return strict; }
            public Map<String, JsonNode> getProviderOptions() { return providerOptions; }
            public List<JsonNode> getInputExamples() { return inputExamples; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String name = "";
                private String description;
                private JsonNode inputSchema = emptyObject();
                private Boolean strict;
                private Map<String, JsonNode> providerOptions;
                private List<JsonNode> inputExamples;

                public Builder name(String v) { this.name = v; return this; }
                public Builder description(String v) { this.description = v; return this; }
                public Builder inputSchema(JsonNode v) { this.inputSchema = v; return this; }
                public Builder strict(Boolean v) { this.strict = v; return this; }
                public Builder providerOptions(Map<String, JsonNode> v) { this.providerOptions = v; return this; }
                public Builder inputExamples(List<JsonNode> v) { this.inputExamples = v; return this; }

                public Function build() {
                    return new Function(name, description, inputSchema, strict, providerOptions, inputExamples);
                }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Function)) return false;
                Function that = (Function) o;
                return Objects.equals(name, that.name)
                    && Objects.equals(description, that.description)
                    && Objects.equals(inputSchema, that.inputSchema)
                    && Objects.equals(strict, that.strict)
                    && Objects.equals(providerOptions, that.providerOptions)
                    && Objects.equals(inputExamples, that.inputExamples);
            }

            @Override
            public int hashCode() {
                return Objects.hash(name, description, inputSchema, strict, providerOptions, inputExamples);
            }
        }

        @JsonTypeName("provider")
        public static class Provider extends Tool {
            @JsonProperty("id") private String id = "";
            @JsonProperty("name") private String name = "";
            @JsonProperty("args") private JsonNode args = emptyObject();

            @JsonCreator
            Provider() {}

            private Provider(String id, String name, JsonNode args) {
                this.id = id;
                this.name = name;
                this.args = args;
            }

            /** Convenience constructor from a {@link ProviderTool}. */
            public static Provider from(ProviderTool tool) {
                return new Provider(tool.id, tool.name, tool.args);
            }

            public String getId() { return id; }
            public String getName() { return name; }
            public JsonNode getArgs() { return args; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String id = "";
                private String name = "";
                private JsonNode args = emptyObject();

                public Builder id(String v) { this.id = v; return this; }
                public Builder name(String v) { this.name = v; return this; }
                public Builder args(JsonNode v) { this.args = v; return this; }

                public Provider build() { return new Provider(id, name, args); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Provider)) return false;
                Provider that = (Provider) o;
                return Objects.equals(id, that.id)
                    && Objects.equals(name, that.name)
                    && Objects.equals(args, that.args);
            }

            @Override
            public int hashCode() {
                return Objects.hash(id, name, args);
            }
        }
    }

    /**
     * How the model should choose tools.
     *
     * Mirrors `ToolChoice.ts`: `"auto" | "none" | "required" | { type: "tool",
     * toolName: "..." }`. This is a mixed tagged/untagged shape (bare strings plus a
     * tagged object), so a custom serializer handles the two forms.
     */
    public abstract static class ToolChoice {
        private ToolChoice() {}

        /** `"auto"` — let the model decide when to call tools. */
        public static final ToolChoice AUTO = new Auto();
        /** `"none"` — never call tools. */
        public static final ToolChoice NONE = new None();
        /** `"required"` — force a tool call. */
        public static final ToolChoice REQUIRED = new Required();

        public static class Auto extends ToolChoice {
            public Auto() {}

            @Override
            public boolean equals(Object o) { return o instanceof Auto; }

            @Override
            public int hashCode() { return 1; }

            @Override
            public String toString() { return "ToolChoice.Auto"; }
        }

        public static class None extends ToolChoice {
            public None() {}

            @Override
            public boolean equals(Object o) { return o instanceof None; }

            @Override
            public int hashCode() { return 2; }

            @Override
            public String toString() { return "ToolChoice.None"; }
        }

        public static class Required extends ToolChoice {
            public Required() {}

            @Override
            public boolean equals(Object o) { return o instanceof Required; }

            @Override
            public int hashCode() { return 3; }

            @Override
            public String toString() { return "ToolChoice.Required"; }
        }

        public static class Tool extends ToolChoice {
            @JsonProperty("type") private final String type = "tool";
            @JsonProperty("toolName") private String toolName = "";

            @JsonCreator
            Tool() {}

            public Tool(String toolName) {
                this.toolName = toolName;
            }

            public String getToolName() { return toolName; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String toolName = "";

                public Builder toolName(String v) { this.toolName = v; return this; }

                public Tool build() { return new Tool(toolName); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Tool)) return false;
                Tool that = (Tool) o;
                return Objects.equals(toolName, that.toolName);
            }

            @Override
            public int hashCode() {
                return Objects.hash(toolName);
            }
        }
    }

    /** Serializes a {@link ToolChoice} as a bare string or a `{"type":"tool",...}` object. */
    public static class ToolChoiceSerializer extends JsonSerializer<ToolChoice> {
        @Override
        public void serialize(ToolChoice value, JsonGenerator gen, SerializerProvider serializers) throws IOException {
            if (value instanceof ToolChoice.Auto) {
                gen.writeString("auto");
            } else if (value instanceof ToolChoice.None) {
                gen.writeString("none");
            } else if (value instanceof ToolChoice.Required) {
                gen.writeString("required");
            } else if (value instanceof ToolChoice.Tool) {
                gen.writeStartObject();
                gen.writeStringField("type", "tool");
                gen.writeStringField("toolName", ((ToolChoice.Tool) value).getToolName());
                gen.writeEndObject();
            } else {
                throw new IOException("Unknown ToolChoice: " + value);
            }
        }
    }

    /** Deserializes a {@link ToolChoice} from a bare string or a tagged object. */
    public static class ToolChoiceDeserializer extends JsonDeserializer<ToolChoice> {
        @Override
        public ToolChoice deserialize(JsonParser p, DeserializationContext ctxt) throws IOException {
            JsonNode node = p.getCodec().readTree(p);
            if (node.isTextual()) {
                switch (node.asText()) {
                    case "auto": return ToolChoice.AUTO;
                    case "none": return ToolChoice.NONE;
                    case "required": return ToolChoice.REQUIRED;
                    default:
                        throw new IOException("Unknown ToolChoice string: '" + node.asText() + "'");
                }
            }
            if (node.isObject()) {
                String type = node.path("type").asText();
                if ("tool".equals(type)) {
                    return new ToolChoice.Tool(node.path("toolName").asText(""));
                }
                throw new IOException("Unknown ToolChoice object: " + node);
            }
            throw new IOException("Unexpected ToolChoice element: " + node);
        }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // ContentPart (multi-part message content).
    //
    // `ContentPart` is internally tagged on `type` (`{"type": "text", ...}`,
    // `{"type": "image", ...}`, ...). A custom serializer dispatches on the
    // `"type"` key and re-injects it on encode.
    // ─────────────────────────────────────────────────────────────────────────────

    /**
     * A part of a multi-part message.
     *
     * Mirrors `ContentPart.ts` (internally tagged on `type`). Shared between
     * {@link ModelMessage} (user-facing) and the provider-facing prompt. Every
     * field has a default and unknown keys are ignored on decode, so future part
     * additions do not break existing clients. `provider_options` is a
     * {@link JsonNode} because it is an opaque `Record<string, JSONObject>`.
     */
    public abstract static class ContentPart {
        private ContentPart() {}

        public static class Text extends ContentPart {
            @JsonProperty("text") private String text = "";
            @JsonProperty("provider_options") private JsonNode providerOptions;

            @JsonCreator
            Text() {}

            private Text(String text, JsonNode providerOptions) {
                this.text = text;
                this.providerOptions = providerOptions;
            }

            public String getText() { return text; }
            public JsonNode getProviderOptions() { return providerOptions; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String text = "";
                private JsonNode providerOptions;

                public Builder text(String v) { this.text = v; return this; }
                public Builder providerOptions(JsonNode v) { this.providerOptions = v; return this; }

                public Text build() { return new Text(text, providerOptions); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Text)) return false;
                Text that = (Text) o;
                return Objects.equals(text, that.text) && Objects.equals(providerOptions, that.providerOptions);
            }

            @Override
            public int hashCode() { return Objects.hash(text, providerOptions); }
        }

        public static class Image extends ContentPart {
            @JsonProperty("image") private List<Integer> image = new ArrayList<>();
            @JsonProperty("media_type") private String mediaType = "";
            @JsonProperty("provider_options") private JsonNode providerOptions;

            @JsonCreator
            Image() {}

            private Image(List<Integer> image, String mediaType, JsonNode providerOptions) {
                this.image = image;
                this.mediaType = mediaType;
                this.providerOptions = providerOptions;
            }

            public List<Integer> getImage() { return image; }
            public String getMediaType() { return mediaType; }
            public JsonNode getProviderOptions() { return providerOptions; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private List<Integer> image = new ArrayList<>();
                private String mediaType = "";
                private JsonNode providerOptions;

                public Builder image(List<Integer> v) { this.image = v; return this; }
                public Builder mediaType(String v) { this.mediaType = v; return this; }
                public Builder providerOptions(JsonNode v) { this.providerOptions = v; return this; }

                public Image build() { return new Image(image, mediaType, providerOptions); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Image)) return false;
                Image that = (Image) o;
                return Objects.equals(image, that.image)
                    && Objects.equals(mediaType, that.mediaType)
                    && Objects.equals(providerOptions, that.providerOptions);
            }

            @Override
            public int hashCode() { return Objects.hash(image, mediaType, providerOptions); }
        }

        public static class File extends ContentPart {
            @JsonProperty("data") private List<Integer> data = new ArrayList<>();
            @JsonProperty("media_type") private String mediaType = "";
            @JsonProperty("filename") private String filename;
            @JsonProperty("provider_options") private JsonNode providerOptions;

            @JsonCreator
            File() {}

            private File(List<Integer> data, String mediaType, String filename, JsonNode providerOptions) {
                this.data = data;
                this.mediaType = mediaType;
                this.filename = filename;
                this.providerOptions = providerOptions;
            }

            public List<Integer> getData() { return data; }
            public String getMediaType() { return mediaType; }
            public String getFilename() { return filename; }
            public JsonNode getProviderOptions() { return providerOptions; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private List<Integer> data = new ArrayList<>();
                private String mediaType = "";
                private String filename;
                private JsonNode providerOptions;

                public Builder data(List<Integer> v) { this.data = v; return this; }
                public Builder mediaType(String v) { this.mediaType = v; return this; }
                public Builder filename(String v) { this.filename = v; return this; }
                public Builder providerOptions(JsonNode v) { this.providerOptions = v; return this; }

                public File build() { return new File(data, mediaType, filename, providerOptions); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof File)) return false;
                File that = (File) o;
                return Objects.equals(data, that.data)
                    && Objects.equals(mediaType, that.mediaType)
                    && Objects.equals(filename, that.filename)
                    && Objects.equals(providerOptions, that.providerOptions);
            }

            @Override
            public int hashCode() { return Objects.hash(data, mediaType, filename, providerOptions); }
        }

        public static class FileBase64 extends ContentPart {
            @JsonProperty("data") private String data = "";
            @JsonProperty("media_type") private String mediaType = "";
            @JsonProperty("filename") private String filename;
            @JsonProperty("provider_options") private JsonNode providerOptions;

            @JsonCreator
            FileBase64() {}

            private FileBase64(String data, String mediaType, String filename, JsonNode providerOptions) {
                this.data = data;
                this.mediaType = mediaType;
                this.filename = filename;
                this.providerOptions = providerOptions;
            }

            public String getData() { return data; }
            public String getMediaType() { return mediaType; }
            public String getFilename() { return filename; }
            public JsonNode getProviderOptions() { return providerOptions; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String data = "";
                private String mediaType = "";
                private String filename;
                private JsonNode providerOptions;

                public Builder data(String v) { this.data = v; return this; }
                public Builder mediaType(String v) { this.mediaType = v; return this; }
                public Builder filename(String v) { this.filename = v; return this; }
                public Builder providerOptions(JsonNode v) { this.providerOptions = v; return this; }

                public FileBase64 build() { return new FileBase64(data, mediaType, filename, providerOptions); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof FileBase64)) return false;
                FileBase64 that = (FileBase64) o;
                return Objects.equals(data, that.data)
                    && Objects.equals(mediaType, that.mediaType)
                    && Objects.equals(filename, that.filename)
                    && Objects.equals(providerOptions, that.providerOptions);
            }

            @Override
            public int hashCode() { return Objects.hash(data, mediaType, filename, providerOptions); }
        }

        public static class FileUrl extends ContentPart {
            @JsonProperty("url") private String url = "";
            @JsonProperty("media_type") private String mediaType = "";
            @JsonProperty("provider_options") private JsonNode providerOptions;

            @JsonCreator
            FileUrl() {}

            private FileUrl(String url, String mediaType, JsonNode providerOptions) {
                this.url = url;
                this.mediaType = mediaType;
                this.providerOptions = providerOptions;
            }

            public String getUrl() { return url; }
            public String getMediaType() { return mediaType; }
            public JsonNode getProviderOptions() { return providerOptions; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String url = "";
                private String mediaType = "";
                private JsonNode providerOptions;

                public Builder url(String v) { this.url = v; return this; }
                public Builder mediaType(String v) { this.mediaType = v; return this; }
                public Builder providerOptions(JsonNode v) { this.providerOptions = v; return this; }

                public FileUrl build() { return new FileUrl(url, mediaType, providerOptions); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof FileUrl)) return false;
                FileUrl that = (FileUrl) o;
                return Objects.equals(url, that.url)
                    && Objects.equals(mediaType, that.mediaType)
                    && Objects.equals(providerOptions, that.providerOptions);
            }

            @Override
            public int hashCode() { return Objects.hash(url, mediaType, providerOptions); }
        }

        public static class FileReference extends ContentPart {
            @JsonProperty("media_type") private String mediaType = "";
            @JsonProperty("reference") private JsonNode reference = emptyObject();
            @JsonProperty("filename") private String filename;
            @JsonProperty("provider_options") private JsonNode providerOptions;

            @JsonCreator
            FileReference() {}

            private FileReference(String mediaType, JsonNode reference, String filename, JsonNode providerOptions) {
                this.mediaType = mediaType;
                this.reference = reference;
                this.filename = filename;
                this.providerOptions = providerOptions;
            }

            public String getMediaType() { return mediaType; }
            public JsonNode getReference() { return reference; }
            public String getFilename() { return filename; }
            public JsonNode getProviderOptions() { return providerOptions; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String mediaType = "";
                private JsonNode reference = emptyObject();
                private String filename;
                private JsonNode providerOptions;

                public Builder mediaType(String v) { this.mediaType = v; return this; }
                public Builder reference(JsonNode v) { this.reference = v; return this; }
                public Builder filename(String v) { this.filename = v; return this; }
                public Builder providerOptions(JsonNode v) { this.providerOptions = v; return this; }

                public FileReference build() {
                    return new FileReference(mediaType, reference, filename, providerOptions);
                }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof FileReference)) return false;
                FileReference that = (FileReference) o;
                return Objects.equals(mediaType, that.mediaType)
                    && Objects.equals(reference, that.reference)
                    && Objects.equals(filename, that.filename)
                    && Objects.equals(providerOptions, that.providerOptions);
            }

            @Override
            public int hashCode() { return Objects.hash(mediaType, reference, filename, providerOptions); }
        }

        public static class Reasoning extends ContentPart {
            @JsonProperty("text") private String text = "";
            @JsonProperty("signature") private String signature;
            @JsonProperty("provider_options") private JsonNode providerOptions;

            @JsonCreator
            Reasoning() {}

            private Reasoning(String text, String signature, JsonNode providerOptions) {
                this.text = text;
                this.signature = signature;
                this.providerOptions = providerOptions;
            }

            public String getText() { return text; }
            public String getSignature() { return signature; }
            public JsonNode getProviderOptions() { return providerOptions; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String text = "";
                private String signature;
                private JsonNode providerOptions;

                public Builder text(String v) { this.text = v; return this; }
                public Builder signature(String v) { this.signature = v; return this; }
                public Builder providerOptions(JsonNode v) { this.providerOptions = v; return this; }

                public Reasoning build() { return new Reasoning(text, signature, providerOptions); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Reasoning)) return false;
                Reasoning that = (Reasoning) o;
                return Objects.equals(text, that.text)
                    && Objects.equals(signature, that.signature)
                    && Objects.equals(providerOptions, that.providerOptions);
            }

            @Override
            public int hashCode() { return Objects.hash(text, signature, providerOptions); }
        }

        public static class ToolCall extends ContentPart {
            @JsonProperty("tool_call_id") private String toolCallId = "";
            @JsonProperty("tool_name") private String toolName = "";
            @JsonProperty("input") private JsonNode input = emptyObject();
            @JsonProperty("provider_options") private JsonNode providerOptions;

            @JsonCreator
            ToolCall() {}

            private ToolCall(String toolCallId, String toolName, JsonNode input, JsonNode providerOptions) {
                this.toolCallId = toolCallId;
                this.toolName = toolName;
                this.input = input;
                this.providerOptions = providerOptions;
            }

            public String getToolCallId() { return toolCallId; }
            public String getToolName() { return toolName; }
            public JsonNode getInput() { return input; }
            public JsonNode getProviderOptions() { return providerOptions; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String toolCallId = "";
                private String toolName = "";
                private JsonNode input = emptyObject();
                private JsonNode providerOptions;

                public Builder toolCallId(String v) { this.toolCallId = v; return this; }
                public Builder toolName(String v) { this.toolName = v; return this; }
                public Builder input(JsonNode v) { this.input = v; return this; }
                public Builder providerOptions(JsonNode v) { this.providerOptions = v; return this; }

                public ToolCall build() { return new ToolCall(toolCallId, toolName, input, providerOptions); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof ToolCall)) return false;
                ToolCall that = (ToolCall) o;
                return Objects.equals(toolCallId, that.toolCallId)
                    && Objects.equals(toolName, that.toolName)
                    && Objects.equals(input, that.input)
                    && Objects.equals(providerOptions, that.providerOptions);
            }

            @Override
            public int hashCode() { return Objects.hash(toolCallId, toolName, input, providerOptions); }
        }

        public static class ToolResult extends ContentPart {
            @JsonProperty("tool_call_id") private String toolCallId = "";
            @JsonProperty("result") private JsonNode result = emptyObject();
            @JsonProperty("tool_name") private String toolName;
            @JsonProperty("is_error") private Boolean isError;
            @JsonProperty("preliminary") private Boolean preliminary;
            @JsonProperty("dynamic") private Boolean dynamic;
            @JsonProperty("provider_options") private JsonNode providerOptions;

            @JsonCreator
            ToolResult() {}

            private ToolResult(String toolCallId, JsonNode result, String toolName, Boolean isError,
                               Boolean preliminary, Boolean dynamic, JsonNode providerOptions) {
                this.toolCallId = toolCallId;
                this.result = result;
                this.toolName = toolName;
                this.isError = isError;
                this.preliminary = preliminary;
                this.dynamic = dynamic;
                this.providerOptions = providerOptions;
            }

            public String getToolCallId() { return toolCallId; }
            public JsonNode getResult() { return result; }
            public String getToolName() { return toolName; }
            public Boolean getIsError() { return isError; }
            public Boolean getPreliminary() { return preliminary; }
            public Boolean getDynamic() { return dynamic; }
            public JsonNode getProviderOptions() { return providerOptions; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String toolCallId = "";
                private JsonNode result = emptyObject();
                private String toolName;
                private Boolean isError;
                private Boolean preliminary;
                private Boolean dynamic;
                private JsonNode providerOptions;

                public Builder toolCallId(String v) { this.toolCallId = v; return this; }
                public Builder result(JsonNode v) { this.result = v; return this; }
                public Builder toolName(String v) { this.toolName = v; return this; }
                public Builder isError(Boolean v) { this.isError = v; return this; }
                public Builder preliminary(Boolean v) { this.preliminary = v; return this; }
                public Builder dynamic(Boolean v) { this.dynamic = v; return this; }
                public Builder providerOptions(JsonNode v) { this.providerOptions = v; return this; }

                public ToolResult build() {
                    return new ToolResult(toolCallId, result, toolName, isError, preliminary, dynamic, providerOptions);
                }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof ToolResult)) return false;
                ToolResult that = (ToolResult) o;
                return Objects.equals(toolCallId, that.toolCallId)
                    && Objects.equals(result, that.result)
                    && Objects.equals(toolName, that.toolName)
                    && Objects.equals(isError, that.isError)
                    && Objects.equals(preliminary, that.preliminary)
                    && Objects.equals(dynamic, that.dynamic)
                    && Objects.equals(providerOptions, that.providerOptions);
            }

            @Override
            public int hashCode() {
                return Objects.hash(toolCallId, result, toolName, isError, preliminary, dynamic, providerOptions);
            }
        }
    }

    /** Custom (de)serializer for {@link ContentPart} — internally tagged on `"type"`. */
    public static class ContentPartSerializer extends JsonSerializer<ContentPart> {
        @Override
        public void serialize(ContentPart value, JsonGenerator gen, SerializerProvider serializers) throws IOException {
            String typeTag = variantTag(value);
            ObjectNode node = (ObjectNode) AimuxJson.INNER_MAPPER.valueToTree(value);
            ObjectNode out = JsonNodeFactory.instance.objectNode();
            out.put("type", typeTag);
            out.setAll(node);
            gen.writeTree(out);
        }

        private static String variantTag(ContentPart value) {
            if (value instanceof ContentPart.Text) return "text";
            if (value instanceof ContentPart.Image) return "image";
            if (value instanceof ContentPart.File) return "file";
            if (value instanceof ContentPart.FileBase64) return "file_base64";
            if (value instanceof ContentPart.FileUrl) return "file_url";
            if (value instanceof ContentPart.FileReference) return "file_reference";
            if (value instanceof ContentPart.Reasoning) return "reasoning";
            if (value instanceof ContentPart.ToolCall) return "tool_call";
            if (value instanceof ContentPart.ToolResult) return "tool_result";
            throw new IllegalArgumentException("Unknown ContentPart: " + value);
        }
    }

    /** Custom (de)serializer for {@link ContentPart}. */
    public static class ContentPartDeserializer extends JsonDeserializer<ContentPart> {
        @Override
        public ContentPart deserialize(JsonParser p, DeserializationContext ctxt) throws IOException {
            JsonNode node = p.getCodec().readTree(p);
            if (!node.isObject()) {
                throw new IOException("ContentPart must be a JSON object, got: " + node);
            }
            String type = node.path("type").asText();
            switch (type) {
                case "text": return AimuxJson.MAPPER.treeToValue(node, ContentPart.Text.class);
                case "image": return AimuxJson.MAPPER.treeToValue(node, ContentPart.Image.class);
                case "file": return AimuxJson.MAPPER.treeToValue(node, ContentPart.File.class);
                case "file_base64": return AimuxJson.MAPPER.treeToValue(node, ContentPart.FileBase64.class);
                case "file_url": return AimuxJson.MAPPER.treeToValue(node, ContentPart.FileUrl.class);
                case "file_reference": return AimuxJson.MAPPER.treeToValue(node, ContentPart.FileReference.class);
                case "reasoning": return AimuxJson.MAPPER.treeToValue(node, ContentPart.Reasoning.class);
                case "tool_call": return AimuxJson.MAPPER.treeToValue(node, ContentPart.ToolCall.class);
                case "tool_result": return AimuxJson.MAPPER.treeToValue(node, ContentPart.ToolResult.class);
                default:
                    throw new IOException("Unknown ContentPart type: '" + type + "'");
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // ModelMessage (prompt side).
    // ─────────────────────────────────────────────────────────────────────────────

    /**
     * Message body: either a simple string or multi-part content.
     *
     * Mirrors `MessageContent.ts`: `string | Array<ContentPart>`. Modeled as a
     * sealed union with a custom (untagged) serializer: a JSON string decodes to
     * {@link MessageContent.Text}, a JSON array decodes to
     * {@link MessageContent.Parts}.
     */
    public abstract static class MessageContent {
        private MessageContent() {}

        public static class Text extends MessageContent {
            private String text = "";

            @JsonCreator
            Text() {}

            public Text(String text) { this.text = text; }

            public String getText() { return text; }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Text)) return false;
                return Objects.equals(text, ((Text) o).text);
            }

            @Override
            public int hashCode() { return Objects.hash(text); }

            @Override
            public String toString() { return "MessageContent.Text(" + text + ")"; }
        }

        public static class Parts extends MessageContent {
            private List<ContentPart> parts = new ArrayList<>();

            @JsonCreator
            Parts() {}

            public Parts(List<ContentPart> parts) { this.parts = parts; }

            public List<ContentPart> getParts() { return parts; }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Parts)) return false;
                return Objects.equals(parts, ((Parts) o).parts);
            }

            @Override
            public int hashCode() { return Objects.hash(parts); }

            @Override
            public String toString() { return "MessageContent.Parts(" + parts + ")"; }
        }
    }

    /** Custom (de)serializer for the untagged {@link MessageContent} union. */
    public static class MessageContentSerializer extends JsonSerializer<MessageContent> {
        @Override
        public void serialize(MessageContent value, JsonGenerator gen, SerializerProvider serializers) throws IOException {
            if (value instanceof MessageContent.Text) {
                gen.writeString(((MessageContent.Text) value).getText());
            } else if (value instanceof MessageContent.Parts) {
                List<ContentPart> parts = ((MessageContent.Parts) value).getParts();
                gen.writeStartArray();
                for (ContentPart part : parts) {
                    gen.writeTree(AimuxJson.MAPPER.valueToTree(part));
                }
                gen.writeEndArray();
            } else {
                throw new IOException("Unknown MessageContent: " + value);
            }
        }
    }

    /** Custom (de)serializer for the untagged {@link MessageContent} union. */
    public static class MessageContentDeserializer extends JsonDeserializer<MessageContent> {
        @Override
        public MessageContent deserialize(JsonParser p, DeserializationContext ctxt) throws IOException {
            JsonNode node = p.getCodec().readTree(p);
            if (node.isTextual()) {
                return new MessageContent.Text(node.asText());
            }
            if (node.isArray()) {
                List<ContentPart> parts = new ArrayList<>();
                for (JsonNode item : node) {
                    parts.add(AimuxJson.MAPPER.treeToValue(item, ContentPart.class));
                }
                return new MessageContent.Parts(parts);
            }
            throw new IOException("MessageContent must be a string or array of ContentPart, got: " + node);
        }
    }

    /**
     * A single user-facing chat message.
     *
     * Mirrors `ModelMessage.ts`: `{ role: Role, content: MessageContent }` where
     * {@link MessageContent} is a string-or-parts union. Use
     * {@link #getContentString()} / {@link #getContentParts()} for ergonomic
     * access, or the static factories to build one.
     */
    public static class ModelMessage {
        @JsonProperty("role") private Role role = Role.USER;
        @JsonProperty("content") private MessageContent content = new MessageContent.Text("");

        @JsonCreator
        ModelMessage() {}

        private ModelMessage(Role role, MessageContent content) {
            this.role = role;
            this.content = content;
        }

        /** Build a message with plain string content (the common case). */
        public static ModelMessage text(Role role, String text) {
            return new ModelMessage(role, new MessageContent.Text(text));
        }

        /** Build a message from a list of {@link ContentPart}s. */
        public static ModelMessage parts(Role role, List<ContentPart> parts) {
            return new ModelMessage(role, new MessageContent.Parts(parts));
        }

        /** Build a message from a pre-built {@link MessageContent}. */
        public static ModelMessage of(Role role, MessageContent content) {
            return new ModelMessage(role, content);
        }

        public Role getRole() { return role; }
        public MessageContent getContent() { return content; }

        /** The content as a plain string, if the message was sent with string content. */
        public String getContentString() {
            return content instanceof MessageContent.Text ? ((MessageContent.Text) content).getText() : null;
        }

        /** The content as a list of parts, if the message was sent with multi-part content. */
        public List<ContentPart> getContentParts() {
            return content instanceof MessageContent.Parts ? ((MessageContent.Parts) content).getParts() : null;
        }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private Role role = Role.USER;
            private MessageContent content = new MessageContent.Text("");

            public Builder role(Role v) { this.role = v; return this; }
            public Builder content(MessageContent v) { this.content = v; return this; }
            public Builder text(String v) { this.content = new MessageContent.Text(v); return this; }
            public Builder parts(List<ContentPart> v) { this.content = new MessageContent.Parts(v); return this; }

            public ModelMessage build() { return new ModelMessage(role, content); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof ModelMessage)) return false;
            ModelMessage that = (ModelMessage) o;
            return role == that.role && Objects.equals(content, that.content);
        }

        @Override
        public int hashCode() {
            return Objects.hash(role, content);
        }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // GenerateTextOptions (input).
    // ─────────────────────────────────────────────────────────────────────────────

    /**
     * Per-call timeout configuration.
     *
     * Mirrors `TimeoutConfiguration.ts`. All values are milliseconds; `null`
     * disables the corresponding limit. A `total` timeout also covers retry
     * backoff and the whole streamed response.
     */
    public static class TimeoutConfiguration {
        @JsonProperty("total_ms") private Long totalMs;
        @JsonProperty("first_chunk_ms") private Long firstChunkMs;
        @JsonProperty("chunk_ms") private Long chunkMs;

        @JsonCreator
        TimeoutConfiguration() {}

        private TimeoutConfiguration(Long totalMs, Long firstChunkMs, Long chunkMs) {
            this.totalMs = totalMs;
            this.firstChunkMs = firstChunkMs;
            this.chunkMs = chunkMs;
        }

        public Long getTotalMs() { return totalMs; }
        public Long getFirstChunkMs() { return firstChunkMs; }
        public Long getChunkMs() { return chunkMs; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private Long totalMs;
            private Long firstChunkMs;
            private Long chunkMs;

            public Builder totalMs(Long v) { this.totalMs = v; return this; }
            public Builder firstChunkMs(Long v) { this.firstChunkMs = v; return this; }
            public Builder chunkMs(Long v) { this.chunkMs = v; return this; }

            public TimeoutConfiguration build() {
                return new TimeoutConfiguration(totalMs, firstChunkMs, chunkMs);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof TimeoutConfiguration)) return false;
            TimeoutConfiguration that = (TimeoutConfiguration) o;
            return Objects.equals(totalMs, that.totalMs)
                && Objects.equals(firstChunkMs, that.firstChunkMs)
                && Objects.equals(chunkMs, that.chunkMs);
        }

        @Override
        public int hashCode() {
            return Objects.hash(totalMs, firstChunkMs, chunkMs);
        }
    }

    /**
     * User-facing options for `generate_text` / `stream_text`.
     *
     * Mirrors `GenerateTextOptions.ts`. Every field is nullable with a `null`
     * default; combined with NON_NULL inclusion, only the fields the caller
     * sets are serialized onto the wire.
     */
    public static class GenerateTextOptions {
        @JsonProperty("max_output_tokens") private Long maxOutputTokens;
        @JsonProperty("temperature") private Double temperature;
        @JsonProperty("stop_sequences") private List<String> stopSequences;
        @JsonProperty("top_p") private Double topP;
        @JsonProperty("top_k") private Long topK;
        @JsonProperty("presence_penalty") private Double presencePenalty;
        @JsonProperty("frequency_penalty") private Double frequencyPenalty;
        @JsonProperty("response_format") private JsonNode responseFormat;
        @JsonProperty("seed") private Long seed;
        @JsonProperty("tools") private List<Tool> tools;
        @JsonProperty("tool_choice") private ToolChoice toolChoice;
        @JsonProperty("headers") private Map<String, String> headers;
        @JsonProperty("provider_options") private Map<String, JsonNode> providerOptions;
        @JsonProperty("reasoning") private ReasoningEffort reasoning;
        @JsonProperty("instructions") private String instructions;
        @JsonProperty("body_overrides") private JsonNode bodyOverrides;
        @JsonProperty("max_retries") private Long maxRetries;
        @JsonProperty("include_raw_chunks") private Boolean includeRawChunks;
        @JsonProperty("timeout") private TimeoutConfiguration timeout;

        @JsonCreator
        GenerateTextOptions() {}

        private GenerateTextOptions(Long maxOutputTokens, Double temperature, List<String> stopSequences,
                                    Double topP, Long topK, Double presencePenalty, Double frequencyPenalty,
                                    JsonNode responseFormat, Long seed, List<Tool> tools, ToolChoice toolChoice,
                                    Map<String, String> headers, Map<String, JsonNode> providerOptions,
                                    ReasoningEffort reasoning, String instructions,
                                    JsonNode bodyOverrides, Long maxRetries, Boolean includeRawChunks,
                                    TimeoutConfiguration timeout) {
            this.maxOutputTokens = maxOutputTokens;
            this.temperature = temperature;
            this.stopSequences = stopSequences;
            this.topP = topP;
            this.topK = topK;
            this.presencePenalty = presencePenalty;
            this.frequencyPenalty = frequencyPenalty;
            this.responseFormat = responseFormat;
            this.seed = seed;
            this.tools = tools;
            this.toolChoice = toolChoice;
            this.headers = headers;
            this.providerOptions = providerOptions;
            this.reasoning = reasoning;
            this.instructions = instructions;
            this.bodyOverrides = bodyOverrides;
            this.maxRetries = maxRetries;
            this.includeRawChunks = includeRawChunks;
            this.timeout = timeout;
        }

        public Long getMaxOutputTokens() { return maxOutputTokens; }
        public Double getTemperature() { return temperature; }
        public List<String> getStopSequences() { return stopSequences; }
        public Double getTopP() { return topP; }
        public Long getTopK() { return topK; }
        public Double getPresencePenalty() { return presencePenalty; }
        public Double getFrequencyPenalty() { return frequencyPenalty; }
        public JsonNode getResponseFormat() { return responseFormat; }
        public Long getSeed() { return seed; }
        public List<Tool> getTools() { return tools; }
        public ToolChoice getToolChoice() { return toolChoice; }
        public Map<String, String> getHeaders() { return headers; }
        public Map<String, JsonNode> getProviderOptions() { return providerOptions; }
        public ReasoningEffort getReasoning() { return reasoning; }
        public String getInstructions() { return instructions; }
        public JsonNode getBodyOverrides() { return bodyOverrides; }
        public Long getMaxRetries() { return maxRetries; }
        public Boolean getIncludeRawChunks() { return includeRawChunks; }
        public TimeoutConfiguration getTimeout() { return timeout; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private Long maxOutputTokens;
            private Double temperature;
            private List<String> stopSequences;
            private Double topP;
            private Long topK;
            private Double presencePenalty;
            private Double frequencyPenalty;
            private JsonNode responseFormat;
            private Long seed;
            private List<Tool> tools;
            private ToolChoice toolChoice;
            private Map<String, String> headers;
            private Map<String, JsonNode> providerOptions;
            private ReasoningEffort reasoning;
            private String instructions;
            private JsonNode bodyOverrides;
            private Long maxRetries;
            private Boolean includeRawChunks;
            private TimeoutConfiguration timeout;

            public Builder maxOutputTokens(Long v) { this.maxOutputTokens = v; return this; }
            public Builder temperature(Double v) { this.temperature = v; return this; }
            public Builder stopSequences(List<String> v) { this.stopSequences = v; return this; }
            public Builder topP(Double v) { this.topP = v; return this; }
            public Builder topK(Long v) { this.topK = v; return this; }
            public Builder presencePenalty(Double v) { this.presencePenalty = v; return this; }
            public Builder frequencyPenalty(Double v) { this.frequencyPenalty = v; return this; }
            public Builder responseFormat(JsonNode v) { this.responseFormat = v; return this; }
            public Builder seed(Long v) { this.seed = v; return this; }
            public Builder tools(List<Tool> v) { this.tools = v; return this; }
            public Builder toolChoice(ToolChoice v) { this.toolChoice = v; return this; }
            public Builder headers(Map<String, String> v) { this.headers = v; return this; }
            public Builder providerOptions(Map<String, JsonNode> v) { this.providerOptions = v; return this; }
            public Builder reasoning(ReasoningEffort v) { this.reasoning = v; return this; }
            public Builder instructions(String v) { this.instructions = v; return this; }
            public Builder bodyOverrides(JsonNode v) { this.bodyOverrides = v; return this; }
            public Builder maxRetries(Long v) { this.maxRetries = v; return this; }
            public Builder includeRawChunks(Boolean v) { this.includeRawChunks = v; return this; }
            public Builder timeout(TimeoutConfiguration v) { this.timeout = v; return this; }

            public GenerateTextOptions build() {
                return new GenerateTextOptions(maxOutputTokens, temperature, stopSequences, topP, topK,
                    presencePenalty, frequencyPenalty, responseFormat, seed, tools, toolChoice, headers,
                    providerOptions, reasoning, instructions, bodyOverrides, maxRetries, includeRawChunks,
                    timeout);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof GenerateTextOptions)) return false;
            GenerateTextOptions that = (GenerateTextOptions) o;
            return Objects.equals(maxOutputTokens, that.maxOutputTokens)
                && Objects.equals(temperature, that.temperature)
                && Objects.equals(stopSequences, that.stopSequences)
                && Objects.equals(topP, that.topP)
                && Objects.equals(topK, that.topK)
                && Objects.equals(presencePenalty, that.presencePenalty)
                && Objects.equals(frequencyPenalty, that.frequencyPenalty)
                && Objects.equals(responseFormat, that.responseFormat)
                && Objects.equals(seed, that.seed)
                && Objects.equals(tools, that.tools)
                && Objects.equals(toolChoice, that.toolChoice)
                && Objects.equals(headers, that.headers)
                && Objects.equals(providerOptions, that.providerOptions)
                && Objects.equals(reasoning, that.reasoning)
                && Objects.equals(instructions, that.instructions)
                && Objects.equals(bodyOverrides, that.bodyOverrides)
                && Objects.equals(maxRetries, that.maxRetries)
                && Objects.equals(includeRawChunks, that.includeRawChunks)
                && Objects.equals(timeout, that.timeout);
        }

        @Override
        public int hashCode() {
            return Objects.hash(maxOutputTokens, temperature, stopSequences, topP, topK, presencePenalty,
                frequencyPenalty, responseFormat, seed, tools, toolChoice, headers, providerOptions, reasoning,
                instructions, bodyOverrides, maxRetries, includeRawChunks, timeout);
        }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // File bytes / file data (shared V4 file types).
    //
    // `FileBytes` and `FileData` are externally-tagged enums
    // (`{"Binary": [...]}`, `{"Data": {"data": ...}}`, ...). Each is modeled with a
    // custom serializer that dispatches on the single tag key.
    // ─────────────────────────────────────────────────────────────────────────────

    /**
     * Either raw bytes or a base64-encoded string.
     *
     * Mirrors `FileBytes.ts`: `{"Binary": Array<number>} | {"Base64": string}`.
     * The binary payload is a list of byte ints (0–255).
     */
    public abstract static class FileBytes {
        private FileBytes() {}

        /** Raw binary bytes (a JSON array of 0–255 ints on the wire). */
        public static class Binary extends FileBytes {
            private List<Integer> data = new ArrayList<>();

            @JsonCreator
            Binary() {}

            public Binary(List<Integer> data) { this.data = data; }

            public List<Integer> getData() { return data; }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Binary)) return false;
                return Objects.equals(data, ((Binary) o).data);
            }

            @Override
            public int hashCode() { return Objects.hash(data); }
        }

        /** A base64-encoded string. */
        public static class Base64 extends FileBytes {
            private String data = "";

            @JsonCreator
            Base64() {}

            public Base64(String data) { this.data = data; }

            public String getData() { return data; }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Base64)) return false;
                return Objects.equals(data, ((Base64) o).data);
            }

            @Override
            public int hashCode() { return Objects.hash(data); }
        }
    }

    /** Custom (de)serializer for {@link FileBytes} — externally tagged. */
    public static class FileBytesSerializer extends JsonSerializer<FileBytes> {
        @Override
        public void serialize(FileBytes value, JsonGenerator gen, SerializerProvider serializers) throws IOException {
            gen.writeStartObject();
            if (value instanceof FileBytes.Binary) {
                gen.writeArrayFieldStart("Binary");
                for (Integer b : ((FileBytes.Binary) value).getData()) {
                    gen.writeNumber(b);
                }
                gen.writeEndArray();
            } else if (value instanceof FileBytes.Base64) {
                gen.writeStringField("Base64", ((FileBytes.Base64) value).getData());
            } else {
                throw new IOException("Unknown FileBytes: " + value);
            }
            gen.writeEndObject();
        }
    }

    /** Custom (de)serializer for {@link FileBytes}. */
    public static class FileBytesDeserializer extends JsonDeserializer<FileBytes> {
        @Override
        public FileBytes deserialize(JsonParser p, DeserializationContext ctxt) throws IOException {
            JsonNode node = p.getCodec().readTree(p);
            if (!node.isObject() || node.size() != 1) {
                throw new IOException("FileBytes must be a single-key externally-tagged object, got: " + node);
            }
            String tag = node.fieldNames().next();
            JsonNode inner = node.get(tag);
            switch (tag) {
                case "Binary": {
                    List<Integer> data = new ArrayList<>();
                    if (inner.isArray()) {
                        for (JsonNode item : inner) {
                            data.add(item.asInt());
                        }
                    }
                    return new FileBytes.Binary(data);
                }
                case "Base64":
                    return new FileBytes.Base64(inner.asText());
                default:
                    throw new IOException("Unknown FileBytes tag: '" + tag + "'");
            }
        }
    }

    /**
     * File data as a tagged discriminated union.
     *
     * Mirrors `FileData.ts`: `{"Data": {"data": FileBytes}} | {"Url": {"url": ...}}
     * | {"Reference": {"reference": {...}}} | {"Text": {"text": ...}}`.
     */
    public abstract static class FileData {
        private FileData() {}

        public static class Data extends FileData {
            private FileBytes data = new FileBytes.Base64("");

            @JsonCreator
            Data() {}

            public Data(FileBytes data) { this.data = data; }

            public FileBytes getData() { return data; }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Data)) return false;
                return Objects.equals(data, ((Data) o).data);
            }

            @Override
            public int hashCode() { return Objects.hash(data); }
        }

        public static class Url extends FileData {
            private String url = "";

            @JsonCreator
            Url() {}

            public Url(String url) { this.url = url; }

            public String getUrl() { return url; }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Url)) return false;
                return Objects.equals(url, ((Url) o).url);
            }

            @Override
            public int hashCode() { return Objects.hash(url); }
        }

        public static class Reference extends FileData {
            private JsonNode reference = emptyObject();

            @JsonCreator
            Reference() {}

            public Reference(JsonNode reference) { this.reference = reference; }

            public JsonNode getReference() { return reference; }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Reference)) return false;
                return Objects.equals(reference, ((Reference) o).reference);
            }

            @Override
            public int hashCode() { return Objects.hash(reference); }
        }

        public static class Text extends FileData {
            private String text = "";

            @JsonCreator
            Text() {}

            public Text(String text) { this.text = text; }

            public String getText() { return text; }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Text)) return false;
                return Objects.equals(text, ((Text) o).text);
            }

            @Override
            public int hashCode() { return Objects.hash(text); }
        }
    }

    /** Custom (de)serializer for {@link FileData} — externally tagged. */
    public static class FileDataSerializer extends JsonSerializer<FileData> {
        @Override
        public void serialize(FileData value, JsonGenerator gen, SerializerProvider serializers) throws IOException {
            gen.writeStartObject();
            if (value instanceof FileData.Data) {
                gen.writeObjectFieldStart("Data");
                gen.writeFieldName("data");
                gen.writeTree(AimuxJson.MAPPER.valueToTree(((FileData.Data) value).getData()));
                gen.writeEndObject();
            } else if (value instanceof FileData.Url) {
                gen.writeObjectField("Url", ((FileData.Url) value).getUrl());
            } else if (value instanceof FileData.Reference) {
                gen.writeObjectField("Reference", ((FileData.Reference) value).getReference());
            } else if (value instanceof FileData.Text) {
                gen.writeObjectField("Text", ((FileData.Text) value).getText());
            } else {
                throw new IOException("Unknown FileData: " + value);
            }
            gen.writeEndObject();
        }
    }

    /** Custom (de)serializer for {@link FileData}. */
    public static class FileDataDeserializer extends JsonDeserializer<FileData> {
        @Override
        public FileData deserialize(JsonParser p, DeserializationContext ctxt) throws IOException {
            JsonNode node = p.getCodec().readTree(p);
            if (!node.isObject() || node.size() != 1) {
                throw new IOException("FileData must be a single-key externally-tagged object, got: " + node);
            }
            String tag = node.fieldNames().next();
            JsonNode inner = node.get(tag);
            JsonNode innerObj = inner.isObject() ? inner : AimuxJson.MAPPER.createObjectNode();
            switch (tag) {
                case "Data":
                    return new FileData.Data(AimuxJson.MAPPER.treeToValue(innerObj.get("data"), FileBytes.class));
                case "Url":
                    return new FileData.Url(inner.asText());
                case "Reference":
                    return new FileData.Reference(innerObj.get("reference") == null ? emptyObject() : innerObj.get("reference"));
                case "Text":
                    return new FileData.Text(inner.asText());
                default:
                    throw new IOException("Unknown FileData tag: '" + tag + "'");
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // GenerateContent / GenerateResult (the `raw` field of GenerateTextResult).
    //
    // `GenerateContent` is externally tagged (`{"Text": {...}}`, `{"ToolCall":
    // {...}}`, ...). Unrecognized variants fall back to GenerateContent.Unknown
    // for forward compatibility (mirroring StreamPart).
    // ─────────────────────────────────────────────────────────────────────────────

    /**
     * A content item in the generation result.
     *
     * Mirrors `GenerateContent.ts` (externally tagged). `provider_metadata` is a
     * {@link JsonNode} (`ProviderMetadata = serde_json::Value`). Every field has a
     * default; the `File` variant has no `filename` (matching Rust).
     */
    public abstract static class GenerateContent {
        private GenerateContent() {}

        public static class Text extends GenerateContent {
            @JsonProperty("text") private String text = "";
            @JsonProperty("provider_metadata") private JsonNode providerMetadata;

            @JsonCreator
            Text() {}

            private Text(String text, JsonNode providerMetadata) {
                this.text = text;
                this.providerMetadata = providerMetadata;
            }

            public String getText() { return text; }
            public JsonNode getProviderMetadata() { return providerMetadata; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String text = "";
                private JsonNode providerMetadata;

                public Builder text(String v) { this.text = v; return this; }
                public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

                public Text build() { return new Text(text, providerMetadata); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Text)) return false;
                Text that = (Text) o;
                return Objects.equals(text, that.text) && Objects.equals(providerMetadata, that.providerMetadata);
            }

            @Override
            public int hashCode() { return Objects.hash(text, providerMetadata); }
        }

        public static class ToolCall extends GenerateContent {
            @JsonProperty("tool_call_id") private String toolCallId = "";
            @JsonProperty("tool_name") private String toolName = "";
            @JsonProperty("input") private JsonNode input = emptyObject();
            @JsonProperty("provider_executed") private Boolean providerExecuted;
            @JsonProperty("dynamic") private Boolean dynamic;
            @JsonProperty("provider_metadata") private JsonNode providerMetadata;

            @JsonCreator
            ToolCall() {}

            private ToolCall(String toolCallId, String toolName, JsonNode input, Boolean providerExecuted,
                             Boolean dynamic, JsonNode providerMetadata) {
                this.toolCallId = toolCallId;
                this.toolName = toolName;
                this.input = input;
                this.providerExecuted = providerExecuted;
                this.dynamic = dynamic;
                this.providerMetadata = providerMetadata;
            }

            public String getToolCallId() { return toolCallId; }
            public String getToolName() { return toolName; }
            public JsonNode getInput() { return input; }
            public Boolean getProviderExecuted() { return providerExecuted; }
            public Boolean getDynamic() { return dynamic; }
            public JsonNode getProviderMetadata() { return providerMetadata; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String toolCallId = "";
                private String toolName = "";
                private JsonNode input = emptyObject();
                private Boolean providerExecuted;
                private Boolean dynamic;
                private JsonNode providerMetadata;

                public Builder toolCallId(String v) { this.toolCallId = v; return this; }
                public Builder toolName(String v) { this.toolName = v; return this; }
                public Builder input(JsonNode v) { this.input = v; return this; }
                public Builder providerExecuted(Boolean v) { this.providerExecuted = v; return this; }
                public Builder dynamic(Boolean v) { this.dynamic = v; return this; }
                public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

                public ToolCall build() {
                    return new ToolCall(toolCallId, toolName, input, providerExecuted, dynamic, providerMetadata);
                }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof ToolCall)) return false;
                ToolCall that = (ToolCall) o;
                return Objects.equals(toolCallId, that.toolCallId)
                    && Objects.equals(toolName, that.toolName)
                    && Objects.equals(input, that.input)
                    && Objects.equals(providerExecuted, that.providerExecuted)
                    && Objects.equals(dynamic, that.dynamic)
                    && Objects.equals(providerMetadata, that.providerMetadata);
            }

            @Override
            public int hashCode() {
                return Objects.hash(toolCallId, toolName, input, providerExecuted, dynamic, providerMetadata);
            }
        }

        public static class Source extends GenerateContent {
            @JsonProperty("id") private String id = "";
            @JsonProperty("source_type") private String sourceType = "";
            @JsonProperty("url") private String url;
            @JsonProperty("title") private String title;
            @JsonProperty("provider_metadata") private JsonNode providerMetadata;

            @JsonCreator
            Source() {}

            private Source(String id, String sourceType, String url, String title, JsonNode providerMetadata) {
                this.id = id;
                this.sourceType = sourceType;
                this.url = url;
                this.title = title;
                this.providerMetadata = providerMetadata;
            }

            public String getId() { return id; }
            public String getSourceType() { return sourceType; }
            public String getUrl() { return url; }
            public String getTitle() { return title; }
            public JsonNode getProviderMetadata() { return providerMetadata; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String id = "";
                private String sourceType = "";
                private String url;
                private String title;
                private JsonNode providerMetadata;

                public Builder id(String v) { this.id = v; return this; }
                public Builder sourceType(String v) { this.sourceType = v; return this; }
                public Builder url(String v) { this.url = v; return this; }
                public Builder title(String v) { this.title = v; return this; }
                public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

                public Source build() { return new Source(id, sourceType, url, title, providerMetadata); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Source)) return false;
                Source that = (Source) o;
                return Objects.equals(id, that.id)
                    && Objects.equals(sourceType, that.sourceType)
                    && Objects.equals(url, that.url)
                    && Objects.equals(title, that.title)
                    && Objects.equals(providerMetadata, that.providerMetadata);
            }

            @Override
            public int hashCode() { return Objects.hash(id, sourceType, url, title, providerMetadata); }
        }

        public static class Reasoning extends GenerateContent {
            @JsonProperty("text") private String text = "";
            @JsonProperty("provider_metadata") private JsonNode providerMetadata;

            @JsonCreator
            Reasoning() {}

            private Reasoning(String text, JsonNode providerMetadata) {
                this.text = text;
                this.providerMetadata = providerMetadata;
            }

            public String getText() { return text; }
            public JsonNode getProviderMetadata() { return providerMetadata; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String text = "";
                private JsonNode providerMetadata;

                public Builder text(String v) { this.text = v; return this; }
                public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

                public Reasoning build() { return new Reasoning(text, providerMetadata); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Reasoning)) return false;
                Reasoning that = (Reasoning) o;
                return Objects.equals(text, that.text) && Objects.equals(providerMetadata, that.providerMetadata);
            }

            @Override
            public int hashCode() { return Objects.hash(text, providerMetadata); }
        }

        public static class File extends GenerateContent {
            @JsonProperty("data") private FileData data = new FileData.Text("");
            @JsonProperty("media_type") private String mediaType = "";
            @JsonProperty("provider_metadata") private JsonNode providerMetadata;

            @JsonCreator
            File() {}

            private File(FileData data, String mediaType, JsonNode providerMetadata) {
                this.data = data;
                this.mediaType = mediaType;
                this.providerMetadata = providerMetadata;
            }

            public FileData getData() { return data; }
            public String getMediaType() { return mediaType; }
            public JsonNode getProviderMetadata() { return providerMetadata; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private FileData data = new FileData.Text("");
                private String mediaType = "";
                private JsonNode providerMetadata;

                public Builder data(FileData v) { this.data = v; return this; }
                public Builder mediaType(String v) { this.mediaType = v; return this; }
                public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

                public File build() { return new File(data, mediaType, providerMetadata); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof File)) return false;
                File that = (File) o;
                return Objects.equals(data, that.data)
                    && Objects.equals(mediaType, that.mediaType)
                    && Objects.equals(providerMetadata, that.providerMetadata);
            }

            @Override
            public int hashCode() { return Objects.hash(data, mediaType, providerMetadata); }
        }

        public static class ToolResult extends GenerateContent {
            @JsonProperty("tool_call_id") private String toolCallId = "";
            @JsonProperty("tool_name") private String toolName = "";
            @JsonProperty("result") private JsonNode result = emptyObject();
            @JsonProperty("is_error") private Boolean isError;
            @JsonProperty("preliminary") private Boolean preliminary;
            @JsonProperty("dynamic") private Boolean dynamic;
            @JsonProperty("provider_metadata") private JsonNode providerMetadata;

            @JsonCreator
            ToolResult() {}

            private ToolResult(String toolCallId, String toolName, JsonNode result, Boolean isError,
                               Boolean preliminary, Boolean dynamic, JsonNode providerMetadata) {
                this.toolCallId = toolCallId;
                this.toolName = toolName;
                this.result = result;
                this.isError = isError;
                this.preliminary = preliminary;
                this.dynamic = dynamic;
                this.providerMetadata = providerMetadata;
            }

            public String getToolCallId() { return toolCallId; }
            public String getToolName() { return toolName; }
            public JsonNode getResult() { return result; }
            public Boolean getIsError() { return isError; }
            public Boolean getPreliminary() { return preliminary; }
            public Boolean getDynamic() { return dynamic; }
            public JsonNode getProviderMetadata() { return providerMetadata; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String toolCallId = "";
                private String toolName = "";
                private JsonNode result = emptyObject();
                private Boolean isError;
                private Boolean preliminary;
                private Boolean dynamic;
                private JsonNode providerMetadata;

                public Builder toolCallId(String v) { this.toolCallId = v; return this; }
                public Builder toolName(String v) { this.toolName = v; return this; }
                public Builder result(JsonNode v) { this.result = v; return this; }
                public Builder isError(Boolean v) { this.isError = v; return this; }
                public Builder preliminary(Boolean v) { this.preliminary = v; return this; }
                public Builder dynamic(Boolean v) { this.dynamic = v; return this; }
                public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

                public ToolResult build() {
                    return new ToolResult(toolCallId, toolName, result, isError, preliminary, dynamic, providerMetadata);
                }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof ToolResult)) return false;
                ToolResult that = (ToolResult) o;
                return Objects.equals(toolCallId, that.toolCallId)
                    && Objects.equals(toolName, that.toolName)
                    && Objects.equals(result, that.result)
                    && Objects.equals(isError, that.isError)
                    && Objects.equals(preliminary, that.preliminary)
                    && Objects.equals(dynamic, that.dynamic)
                    && Objects.equals(providerMetadata, that.providerMetadata);
            }

            @Override
            public int hashCode() {
                return Objects.hash(toolCallId, toolName, result, isError, preliminary, dynamic, providerMetadata);
            }
        }

        /** Fallback for variants introduced after this wrapper was written. */
        public static class Unknown extends GenerateContent {
            private String tag;
            private JsonNode data;

            @JsonCreator
            Unknown() {}

            public Unknown(String tag, JsonNode data) {
                this.tag = tag;
                this.data = data;
            }

            public String getTag() { return tag; }
            public JsonNode getData() { return data; }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Unknown)) return false;
                Unknown that = (Unknown) o;
                return Objects.equals(tag, that.tag) && Objects.equals(data, that.data);
            }

            @Override
            public int hashCode() { return Objects.hash(tag, data); }
        }
    }

    /** The externally-tagged variant name for this {@link GenerateContent} (e.g. "Text", "ToolCall"). */
    public static String variantTag(GenerateContent value) {
        if (value instanceof GenerateContent.Text) return "Text";
        if (value instanceof GenerateContent.ToolCall) return "ToolCall";
        if (value instanceof GenerateContent.Source) return "Source";
        if (value instanceof GenerateContent.Reasoning) return "Reasoning";
        if (value instanceof GenerateContent.File) return "File";
        if (value instanceof GenerateContent.ToolResult) return "ToolResult";
        if (value instanceof GenerateContent.Unknown) return ((GenerateContent.Unknown) value).getTag();
        throw new IllegalArgumentException("Unknown GenerateContent: " + value);
    }

    /** Custom (de)serializer for {@link GenerateContent} — externally tagged. */
    public static class GenerateContentSerializer extends JsonSerializer<GenerateContent> {
        @Override
        public void serialize(GenerateContent value, JsonGenerator gen, SerializerProvider serializers) throws IOException {
            String tag = variantTag(value);
            if (value instanceof GenerateContent.Unknown) {
                ObjectNode out = JsonNodeFactory.instance.objectNode();
                out.set(tag, ((GenerateContent.Unknown) value).getData());
                gen.writeTree(out);
                return;
            }
            ObjectNode node = (ObjectNode) AimuxJson.INNER_MAPPER.valueToTree(value);
            ObjectNode out = JsonNodeFactory.instance.objectNode();
            out.set(tag, node);
            gen.writeTree(out);
        }
    }

    /** Custom (de)serializer for {@link GenerateContent}. */
    public static class GenerateContentDeserializer extends JsonDeserializer<GenerateContent> {
        @Override
        public GenerateContent deserialize(JsonParser p, DeserializationContext ctxt) throws IOException {
            JsonNode node = p.getCodec().readTree(p);
            if (!node.isObject() || node.size() != 1) {
                throw new IOException("GenerateContent must be a single-key externally-tagged object, got: " + node);
            }
            String tag = node.fieldNames().next();
            JsonNode inner = node.get(tag);
            JsonNode innerObj = inner.isObject() ? inner : AimuxJson.MAPPER.createObjectNode();
            switch (tag) {
                case "Text": return AimuxJson.MAPPER.treeToValue(innerObj, GenerateContent.Text.class);
                case "ToolCall": return AimuxJson.MAPPER.treeToValue(innerObj, GenerateContent.ToolCall.class);
                case "Source": return AimuxJson.MAPPER.treeToValue(innerObj, GenerateContent.Source.class);
                case "Reasoning": return AimuxJson.MAPPER.treeToValue(innerObj, GenerateContent.Reasoning.class);
                case "File": return AimuxJson.MAPPER.treeToValue(innerObj, GenerateContent.File.class);
                case "ToolResult": return AimuxJson.MAPPER.treeToValue(innerObj, GenerateContent.ToolResult.class);
                default: return new GenerateContent.Unknown(tag, inner);
            }
        }
    }

    /**
     * Result of `LanguageModel::do_generate` (non-streaming) — the raw provider
     * result surfaced via {@link GenerateTextResult#getRaw()}.
     *
     * Mirrors `GenerateResult.ts`. `content` holds typed {@link GenerateContent}
     * items.
     */
    public static class GenerateResult {
        @JsonProperty("content") private List<GenerateContent> content = new ArrayList<>();
        @JsonProperty("finish_reason") private FinishReason finishReason = new FinishReason();
        @JsonProperty("usage") private Usage usage = new Usage();
        @JsonProperty("warnings") private List<JsonNode> warnings = new ArrayList<>();
        @JsonProperty("provider_metadata") private JsonNode providerMetadata;
        @JsonProperty("response") private ResponseMetadata response = new ResponseMetadata();
        @JsonProperty("request_body") private JsonNode requestBody;
        @JsonProperty("response_headers") private Map<String, String> responseHeaders;

        @JsonCreator
        GenerateResult() {}

        private GenerateResult(List<GenerateContent> content, FinishReason finishReason, Usage usage,
                               List<JsonNode> warnings, JsonNode providerMetadata, ResponseMetadata response,
                               JsonNode requestBody, Map<String, String> responseHeaders) {
            this.content = content;
            this.finishReason = finishReason;
            this.usage = usage;
            this.warnings = warnings;
            this.providerMetadata = providerMetadata;
            this.response = response;
            this.requestBody = requestBody;
            this.responseHeaders = responseHeaders;
        }

        public List<GenerateContent> getContent() { return content; }
        public FinishReason getFinishReason() { return finishReason; }
        public Usage getUsage() { return usage; }
        public List<JsonNode> getWarnings() { return warnings; }
        public JsonNode getProviderMetadata() { return providerMetadata; }
        public ResponseMetadata getResponse() { return response; }
        public JsonNode getRequestBody() { return requestBody; }
        public Map<String, String> getResponseHeaders() { return responseHeaders; }

        /** Names of the variant tags present in {@link #getContent()} (e.g. "Text", "ToolCall"). */
        public List<String> getContentVariantTags() {
            List<String> tags = new ArrayList<>();
            for (GenerateContent item : content) {
                tags.add(variantTag(item));
            }
            return tags;
        }

        /** `true` if any content item carries the given externally-tagged variant. */
        public boolean hasContentVariant(String tag) {
            for (GenerateContent item : content) {
                if (variantTag(item).equals(tag)) return true;
            }
            return false;
        }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private List<GenerateContent> content = new ArrayList<>();
            private FinishReason finishReason = new FinishReason();
            private Usage usage = new Usage();
            private List<JsonNode> warnings = new ArrayList<>();
            private JsonNode providerMetadata;
            private ResponseMetadata response = new ResponseMetadata();
            private JsonNode requestBody;
            private Map<String, String> responseHeaders;

            public Builder content(List<GenerateContent> v) { this.content = v; return this; }
            public Builder finishReason(FinishReason v) { this.finishReason = v; return this; }
            public Builder usage(Usage v) { this.usage = v; return this; }
            public Builder warnings(List<JsonNode> v) { this.warnings = v; return this; }
            public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }
            public Builder response(ResponseMetadata v) { this.response = v; return this; }
            public Builder requestBody(JsonNode v) { this.requestBody = v; return this; }
            public Builder responseHeaders(Map<String, String> v) { this.responseHeaders = v; return this; }

            public GenerateResult build() {
                return new GenerateResult(content, finishReason, usage, warnings, providerMetadata, response,
                    requestBody, responseHeaders);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof GenerateResult)) return false;
            GenerateResult that = (GenerateResult) o;
            return Objects.equals(content, that.content)
                && Objects.equals(finishReason, that.finishReason)
                && Objects.equals(usage, that.usage)
                && Objects.equals(warnings, that.warnings)
                && Objects.equals(providerMetadata, that.providerMetadata)
                && Objects.equals(response, that.response)
                && Objects.equals(requestBody, that.requestBody)
                && Objects.equals(responseHeaders, that.responseHeaders);
        }

        @Override
        public int hashCode() {
            return Objects.hash(content, finishReason, usage, warnings, providerMetadata, response, requestBody,
                responseHeaders);
        }
    }

    /**
     * Result of `generate_text` (user-facing).
     *
     * Mirrors `GenerateTextResult.ts`.
     */
    public static class GenerateTextResult {
        @JsonProperty("text") private String text = "";
        @JsonProperty("tool_calls") private List<ToolCall> toolCalls = new ArrayList<>();
        @JsonProperty("finish_reason") private FinishReason finishReason = new FinishReason();
        @JsonProperty("usage") private Usage usage = new Usage();
        @JsonProperty("warnings") private List<JsonNode> warnings = new ArrayList<>();
        @JsonProperty("raw") private GenerateResult raw = new GenerateResult();

        @JsonCreator
        GenerateTextResult() {}

        private GenerateTextResult(String text, List<ToolCall> toolCalls, FinishReason finishReason, Usage usage,
                                   List<JsonNode> warnings, GenerateResult raw) {
            this.text = text;
            this.toolCalls = toolCalls;
            this.finishReason = finishReason;
            this.usage = usage;
            this.warnings = warnings;
            this.raw = raw;
        }

        public String getText() { return text; }
        public List<ToolCall> getToolCalls() { return toolCalls; }
        public FinishReason getFinishReason() { return finishReason; }
        public Usage getUsage() { return usage; }
        public List<JsonNode> getWarnings() { return warnings; }
        public GenerateResult getRaw() { return raw; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String text = "";
            private List<ToolCall> toolCalls = new ArrayList<>();
            private FinishReason finishReason = new FinishReason();
            private Usage usage = new Usage();
            private List<JsonNode> warnings = new ArrayList<>();
            private GenerateResult raw = new GenerateResult();

            public Builder text(String v) { this.text = v; return this; }
            public Builder toolCalls(List<ToolCall> v) { this.toolCalls = v; return this; }
            public Builder finishReason(FinishReason v) { this.finishReason = v; return this; }
            public Builder usage(Usage v) { this.usage = v; return this; }
            public Builder warnings(List<JsonNode> v) { this.warnings = v; return this; }
            public Builder raw(GenerateResult v) { this.raw = v; return this; }

            public GenerateTextResult build() {
                return new GenerateTextResult(text, toolCalls, finishReason, usage, warnings, raw);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof GenerateTextResult)) return false;
            GenerateTextResult that = (GenerateTextResult) o;
            return Objects.equals(text, that.text)
                && Objects.equals(toolCalls, that.toolCalls)
                && Objects.equals(finishReason, that.finishReason)
                && Objects.equals(usage, that.usage)
                && Objects.equals(warnings, that.warnings)
                && Objects.equals(raw, that.raw);
        }

        @Override
        public int hashCode() {
            return Objects.hash(text, toolCalls, finishReason, usage, warnings, raw);
        }
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // StreamPart (the streaming chunk type).
    //
    // `StreamPart` is externally tagged (`{"TextDelta": {...}}`, `{"ToolCall":
    // {...}}`, ...). Unrecognized variants fall back to StreamPart.Unknown for
    // forward compatibility.
    // ─────────────────────────────────────────────────────────────────────────────

    /**
     * A single streaming chunk.
     *
     * Mirrors `StreamPart.ts` (externally tagged).
     */
    public abstract static class StreamPart {
        private StreamPart() {}

        public static class TextStart extends StreamPart {
            @JsonProperty("id") private String id = "";
            @JsonProperty("provider_metadata") private JsonNode providerMetadata;

            @JsonCreator
            TextStart() {}

            private TextStart(String id, JsonNode providerMetadata) {
                this.id = id;
                this.providerMetadata = providerMetadata;
            }

            public String getId() { return id; }
            public JsonNode getProviderMetadata() { return providerMetadata; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String id = "";
                private JsonNode providerMetadata;

                public Builder id(String v) { this.id = v; return this; }
                public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

                public TextStart build() { return new TextStart(id, providerMetadata); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof TextStart)) return false;
                TextStart that = (TextStart) o;
                return Objects.equals(id, that.id) && Objects.equals(providerMetadata, that.providerMetadata);
            }

            @Override
            public int hashCode() { return Objects.hash(id, providerMetadata); }
        }

        public static class TextDelta extends StreamPart {
            @JsonProperty("id") private String id = "";
            @JsonProperty("delta") private String delta = "";
            @JsonProperty("provider_metadata") private JsonNode providerMetadata;

            @JsonCreator
            TextDelta() {}

            private TextDelta(String id, String delta, JsonNode providerMetadata) {
                this.id = id;
                this.delta = delta;
                this.providerMetadata = providerMetadata;
            }

            public String getId() { return id; }
            public String getDelta() { return delta; }
            public JsonNode getProviderMetadata() { return providerMetadata; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String id = "";
                private String delta = "";
                private JsonNode providerMetadata;

                public Builder id(String v) { this.id = v; return this; }
                public Builder delta(String v) { this.delta = v; return this; }
                public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

                public TextDelta build() { return new TextDelta(id, delta, providerMetadata); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof TextDelta)) return false;
                TextDelta that = (TextDelta) o;
                return Objects.equals(id, that.id)
                    && Objects.equals(delta, that.delta)
                    && Objects.equals(providerMetadata, that.providerMetadata);
            }

            @Override
            public int hashCode() { return Objects.hash(id, delta, providerMetadata); }
        }

        public static class TextEnd extends StreamPart {
            @JsonProperty("id") private String id = "";
            @JsonProperty("provider_metadata") private JsonNode providerMetadata;

            @JsonCreator
            TextEnd() {}

            private TextEnd(String id, JsonNode providerMetadata) {
                this.id = id;
                this.providerMetadata = providerMetadata;
            }

            public String getId() { return id; }
            public JsonNode getProviderMetadata() { return providerMetadata; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String id = "";
                private JsonNode providerMetadata;

                public Builder id(String v) { this.id = v; return this; }
                public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

                public TextEnd build() { return new TextEnd(id, providerMetadata); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof TextEnd)) return false;
                TextEnd that = (TextEnd) o;
                return Objects.equals(id, that.id) && Objects.equals(providerMetadata, that.providerMetadata);
            }

            @Override
            public int hashCode() { return Objects.hash(id, providerMetadata); }
        }

        public static class StreamStart extends StreamPart {
            @JsonProperty("warnings") private List<JsonNode> warnings = new ArrayList<>();

            @JsonCreator
            StreamStart() {}

            private StreamStart(List<JsonNode> warnings) { this.warnings = warnings; }

            public List<JsonNode> getWarnings() { return warnings; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private List<JsonNode> warnings = new ArrayList<>();

                public Builder warnings(List<JsonNode> v) { this.warnings = v; return this; }

                public StreamStart build() { return new StreamStart(warnings); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof StreamStart)) return false;
                return Objects.equals(warnings, ((StreamStart) o).warnings);
            }

            @Override
            public int hashCode() { return Objects.hash(warnings); }
        }

        public static class Finish extends StreamPart {
            @JsonProperty("finish_reason") private FinishReason finishReason = new FinishReason();
            @JsonProperty("usage") private Usage usage = new Usage();
            @JsonProperty("provider_metadata") private JsonNode providerMetadata;

            @JsonCreator
            Finish() {}

            private Finish(FinishReason finishReason, Usage usage, JsonNode providerMetadata) {
                this.finishReason = finishReason;
                this.usage = usage;
                this.providerMetadata = providerMetadata;
            }

            public FinishReason getFinishReason() { return finishReason; }
            public Usage getUsage() { return usage; }
            public JsonNode getProviderMetadata() { return providerMetadata; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private FinishReason finishReason = new FinishReason();
                private Usage usage = new Usage();
                private JsonNode providerMetadata;

                public Builder finishReason(FinishReason v) { this.finishReason = v; return this; }
                public Builder usage(Usage v) { this.usage = v; return this; }
                public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

                public Finish build() { return new Finish(finishReason, usage, providerMetadata); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Finish)) return false;
                Finish that = (Finish) o;
                return Objects.equals(finishReason, that.finishReason)
                    && Objects.equals(usage, that.usage)
                    && Objects.equals(providerMetadata, that.providerMetadata);
            }

            @Override
            public int hashCode() { return Objects.hash(finishReason, usage, providerMetadata); }
        }

        public static class ToolInputStart extends StreamPart {
            @JsonProperty("id") private String id = "";
            @JsonProperty("tool_name") private String toolName = "";
            @JsonProperty("provider_executed") private Boolean providerExecuted;
            @JsonProperty("dynamic") private Boolean dynamic;
            @JsonProperty("title") private String title;
            @JsonProperty("provider_metadata") private JsonNode providerMetadata;

            @JsonCreator
            ToolInputStart() {}

            private ToolInputStart(String id, String toolName, Boolean providerExecuted, Boolean dynamic,
                                   String title, JsonNode providerMetadata) {
                this.id = id;
                this.toolName = toolName;
                this.providerExecuted = providerExecuted;
                this.dynamic = dynamic;
                this.title = title;
                this.providerMetadata = providerMetadata;
            }

            public String getId() { return id; }
            public String getToolName() { return toolName; }
            public Boolean getProviderExecuted() { return providerExecuted; }
            public Boolean getDynamic() { return dynamic; }
            public String getTitle() { return title; }
            public JsonNode getProviderMetadata() { return providerMetadata; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String id = "";
                private String toolName = "";
                private Boolean providerExecuted;
                private Boolean dynamic;
                private String title;
                private JsonNode providerMetadata;

                public Builder id(String v) { this.id = v; return this; }
                public Builder toolName(String v) { this.toolName = v; return this; }
                public Builder providerExecuted(Boolean v) { this.providerExecuted = v; return this; }
                public Builder dynamic(Boolean v) { this.dynamic = v; return this; }
                public Builder title(String v) { this.title = v; return this; }
                public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

                public ToolInputStart build() {
                    return new ToolInputStart(id, toolName, providerExecuted, dynamic, title, providerMetadata);
                }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof ToolInputStart)) return false;
                ToolInputStart that = (ToolInputStart) o;
                return Objects.equals(id, that.id)
                    && Objects.equals(toolName, that.toolName)
                    && Objects.equals(providerExecuted, that.providerExecuted)
                    && Objects.equals(dynamic, that.dynamic)
                    && Objects.equals(title, that.title)
                    && Objects.equals(providerMetadata, that.providerMetadata);
            }

            @Override
            public int hashCode() {
                return Objects.hash(id, toolName, providerExecuted, dynamic, title, providerMetadata);
            }
        }

        public static class ToolInputDelta extends StreamPart {
            @JsonProperty("id") private String id = "";
            @JsonProperty("delta") private String delta = "";
            @JsonProperty("provider_metadata") private JsonNode providerMetadata;

            @JsonCreator
            ToolInputDelta() {}

            private ToolInputDelta(String id, String delta, JsonNode providerMetadata) {
                this.id = id;
                this.delta = delta;
                this.providerMetadata = providerMetadata;
            }

            public String getId() { return id; }
            public String getDelta() { return delta; }
            public JsonNode getProviderMetadata() { return providerMetadata; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String id = "";
                private String delta = "";
                private JsonNode providerMetadata;

                public Builder id(String v) { this.id = v; return this; }
                public Builder delta(String v) { this.delta = v; return this; }
                public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

                public ToolInputDelta build() { return new ToolInputDelta(id, delta, providerMetadata); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof ToolInputDelta)) return false;
                ToolInputDelta that = (ToolInputDelta) o;
                return Objects.equals(id, that.id)
                    && Objects.equals(delta, that.delta)
                    && Objects.equals(providerMetadata, that.providerMetadata);
            }

            @Override
            public int hashCode() { return Objects.hash(id, delta, providerMetadata); }
        }

        public static class ToolInputEnd extends StreamPart {
            @JsonProperty("id") private String id = "";
            @JsonProperty("provider_metadata") private JsonNode providerMetadata;

            @JsonCreator
            ToolInputEnd() {}

            private ToolInputEnd(String id, JsonNode providerMetadata) {
                this.id = id;
                this.providerMetadata = providerMetadata;
            }

            public String getId() { return id; }
            public JsonNode getProviderMetadata() { return providerMetadata; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String id = "";
                private JsonNode providerMetadata;

                public Builder id(String v) { this.id = v; return this; }
                public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

                public ToolInputEnd build() { return new ToolInputEnd(id, providerMetadata); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof ToolInputEnd)) return false;
                ToolInputEnd that = (ToolInputEnd) o;
                return Objects.equals(id, that.id) && Objects.equals(providerMetadata, that.providerMetadata);
            }

            @Override
            public int hashCode() { return Objects.hash(id, providerMetadata); }
        }

        public static class ToolCall extends StreamPart {
            @JsonProperty("tool_call_id") private String toolCallId = "";
            @JsonProperty("tool_name") private String toolName = "";
            @JsonProperty("input") private JsonNode input = emptyObject();
            @JsonProperty("provider_executed") private Boolean providerExecuted;
            @JsonProperty("dynamic") private Boolean dynamic;
            @JsonProperty("provider_metadata") private JsonNode providerMetadata;

            @JsonCreator
            ToolCall() {}

            private ToolCall(String toolCallId, String toolName, JsonNode input, Boolean providerExecuted,
                             Boolean dynamic, JsonNode providerMetadata) {
                this.toolCallId = toolCallId;
                this.toolName = toolName;
                this.input = input;
                this.providerExecuted = providerExecuted;
                this.dynamic = dynamic;
                this.providerMetadata = providerMetadata;
            }

            public String getToolCallId() { return toolCallId; }
            public String getToolName() { return toolName; }
            public JsonNode getInput() { return input; }
            public Boolean getProviderExecuted() { return providerExecuted; }
            public Boolean getDynamic() { return dynamic; }
            public JsonNode getProviderMetadata() { return providerMetadata; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String toolCallId = "";
                private String toolName = "";
                private JsonNode input = emptyObject();
                private Boolean providerExecuted;
                private Boolean dynamic;
                private JsonNode providerMetadata;

                public Builder toolCallId(String v) { this.toolCallId = v; return this; }
                public Builder toolName(String v) { this.toolName = v; return this; }
                public Builder input(JsonNode v) { this.input = v; return this; }
                public Builder providerExecuted(Boolean v) { this.providerExecuted = v; return this; }
                public Builder dynamic(Boolean v) { this.dynamic = v; return this; }
                public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

                public ToolCall build() {
                    return new ToolCall(toolCallId, toolName, input, providerExecuted, dynamic, providerMetadata);
                }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof ToolCall)) return false;
                ToolCall that = (ToolCall) o;
                return Objects.equals(toolCallId, that.toolCallId)
                    && Objects.equals(toolName, that.toolName)
                    && Objects.equals(input, that.input)
                    && Objects.equals(providerExecuted, that.providerExecuted)
                    && Objects.equals(dynamic, that.dynamic)
                    && Objects.equals(providerMetadata, that.providerMetadata);
            }

            @Override
            public int hashCode() {
                return Objects.hash(toolCallId, toolName, input, providerExecuted, dynamic, providerMetadata);
            }
        }

        public static class ToolResult extends StreamPart {
            @JsonProperty("tool_call_id") private String toolCallId = "";
            @JsonProperty("tool_name") private String toolName = "";
            @JsonProperty("result") private JsonNode result = emptyObject();
            @JsonProperty("is_error") private Boolean isError;
            @JsonProperty("preliminary") private Boolean preliminary;
            @JsonProperty("dynamic") private Boolean dynamic;
            @JsonProperty("provider_metadata") private JsonNode providerMetadata;

            @JsonCreator
            ToolResult() {}

            private ToolResult(String toolCallId, String toolName, JsonNode result, Boolean isError,
                               Boolean preliminary, Boolean dynamic, JsonNode providerMetadata) {
                this.toolCallId = toolCallId;
                this.toolName = toolName;
                this.result = result;
                this.isError = isError;
                this.preliminary = preliminary;
                this.dynamic = dynamic;
                this.providerMetadata = providerMetadata;
            }

            public String getToolCallId() { return toolCallId; }
            public String getToolName() { return toolName; }
            public JsonNode getResult() { return result; }
            public Boolean getIsError() { return isError; }
            public Boolean getPreliminary() { return preliminary; }
            public Boolean getDynamic() { return dynamic; }
            public JsonNode getProviderMetadata() { return providerMetadata; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String toolCallId = "";
                private String toolName = "";
                private JsonNode result = emptyObject();
                private Boolean isError;
                private Boolean preliminary;
                private Boolean dynamic;
                private JsonNode providerMetadata;

                public Builder toolCallId(String v) { this.toolCallId = v; return this; }
                public Builder toolName(String v) { this.toolName = v; return this; }
                public Builder result(JsonNode v) { this.result = v; return this; }
                public Builder isError(Boolean v) { this.isError = v; return this; }
                public Builder preliminary(Boolean v) { this.preliminary = v; return this; }
                public Builder dynamic(Boolean v) { this.dynamic = v; return this; }
                public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

                public ToolResult build() {
                    return new ToolResult(toolCallId, toolName, result, isError, preliminary, dynamic, providerMetadata);
                }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof ToolResult)) return false;
                ToolResult that = (ToolResult) o;
                return Objects.equals(toolCallId, that.toolCallId)
                    && Objects.equals(toolName, that.toolName)
                    && Objects.equals(result, that.result)
                    && Objects.equals(isError, that.isError)
                    && Objects.equals(preliminary, that.preliminary)
                    && Objects.equals(dynamic, that.dynamic)
                    && Objects.equals(providerMetadata, that.providerMetadata);
            }

            @Override
            public int hashCode() {
                return Objects.hash(toolCallId, toolName, result, isError, preliminary, dynamic, providerMetadata);
            }
        }

        /** A file generated by the model (e.g. an image or document). */
        public static class File extends StreamPart {
            @JsonProperty("data") private JsonNode data = emptyObject();
            @JsonProperty("media_type") private String mediaType = "";
            @JsonProperty("provider_metadata") private JsonNode providerMetadata;

            @JsonCreator
            File() {}

            private File(JsonNode data, String mediaType, JsonNode providerMetadata) {
                this.data = data;
                this.mediaType = mediaType;
                this.providerMetadata = providerMetadata;
            }

            public JsonNode getData() { return data; }
            public String getMediaType() { return mediaType; }
            public JsonNode getProviderMetadata() { return providerMetadata; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private JsonNode data = emptyObject();
                private String mediaType = "";
                private JsonNode providerMetadata;

                public Builder data(JsonNode v) { this.data = v; return this; }
                public Builder mediaType(String v) { this.mediaType = v; return this; }
                public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

                public File build() { return new File(data, mediaType, providerMetadata); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof File)) return false;
                File that = (File) o;
                return Objects.equals(data, that.data)
                    && Objects.equals(mediaType, that.mediaType)
                    && Objects.equals(providerMetadata, that.providerMetadata);
            }

            @Override
            public int hashCode() { return Objects.hash(data, mediaType, providerMetadata); }
        }

        public static class ReasoningStart extends StreamPart {
            @JsonProperty("id") private String id = "";
            @JsonProperty("provider_metadata") private JsonNode providerMetadata;

            @JsonCreator
            ReasoningStart() {}

            private ReasoningStart(String id, JsonNode providerMetadata) {
                this.id = id;
                this.providerMetadata = providerMetadata;
            }

            public String getId() { return id; }
            public JsonNode getProviderMetadata() { return providerMetadata; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String id = "";
                private JsonNode providerMetadata;

                public Builder id(String v) { this.id = v; return this; }
                public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

                public ReasoningStart build() { return new ReasoningStart(id, providerMetadata); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof ReasoningStart)) return false;
                ReasoningStart that = (ReasoningStart) o;
                return Objects.equals(id, that.id) && Objects.equals(providerMetadata, that.providerMetadata);
            }

            @Override
            public int hashCode() { return Objects.hash(id, providerMetadata); }
        }

        public static class ReasoningDelta extends StreamPart {
            @JsonProperty("id") private String id = "";
            @JsonProperty("delta") private String delta = "";
            @JsonProperty("provider_metadata") private JsonNode providerMetadata;

            @JsonCreator
            ReasoningDelta() {}

            private ReasoningDelta(String id, String delta, JsonNode providerMetadata) {
                this.id = id;
                this.delta = delta;
                this.providerMetadata = providerMetadata;
            }

            public String getId() { return id; }
            public String getDelta() { return delta; }
            public JsonNode getProviderMetadata() { return providerMetadata; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String id = "";
                private String delta = "";
                private JsonNode providerMetadata;

                public Builder id(String v) { this.id = v; return this; }
                public Builder delta(String v) { this.delta = v; return this; }
                public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

                public ReasoningDelta build() { return new ReasoningDelta(id, delta, providerMetadata); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof ReasoningDelta)) return false;
                ReasoningDelta that = (ReasoningDelta) o;
                return Objects.equals(id, that.id)
                    && Objects.equals(delta, that.delta)
                    && Objects.equals(providerMetadata, that.providerMetadata);
            }

            @Override
            public int hashCode() { return Objects.hash(id, delta, providerMetadata); }
        }

        public static class ReasoningEnd extends StreamPart {
            @JsonProperty("id") private String id = "";
            @JsonProperty("provider_metadata") private JsonNode providerMetadata;

            @JsonCreator
            ReasoningEnd() {}

            private ReasoningEnd(String id, JsonNode providerMetadata) {
                this.id = id;
                this.providerMetadata = providerMetadata;
            }

            public String getId() { return id; }
            public JsonNode getProviderMetadata() { return providerMetadata; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String id = "";
                private JsonNode providerMetadata;

                public Builder id(String v) { this.id = v; return this; }
                public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

                public ReasoningEnd build() { return new ReasoningEnd(id, providerMetadata); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof ReasoningEnd)) return false;
                ReasoningEnd that = (ReasoningEnd) o;
                return Objects.equals(id, that.id) && Objects.equals(providerMetadata, that.providerMetadata);
            }

            @Override
            public int hashCode() { return Objects.hash(id, providerMetadata); }
        }

        public static class ResponseMetadata extends StreamPart {
            @JsonProperty("id") private String id;
            @JsonProperty("timestamp") private String timestamp;
            @JsonProperty("model_id") private String modelId;

            @JsonCreator
            ResponseMetadata() {}

            private ResponseMetadata(String id, String timestamp, String modelId) {
                this.id = id;
                this.timestamp = timestamp;
                this.modelId = modelId;
            }

            public String getId() { return id; }
            public String getTimestamp() { return timestamp; }
            public String getModelId() { return modelId; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String id;
                private String timestamp;
                private String modelId;

                public Builder id(String v) { this.id = v; return this; }
                public Builder timestamp(String v) { this.timestamp = v; return this; }
                public Builder modelId(String v) { this.modelId = v; return this; }

                public ResponseMetadata build() { return new ResponseMetadata(id, timestamp, modelId); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof ResponseMetadata)) return false;
                ResponseMetadata that = (ResponseMetadata) o;
                return Objects.equals(id, that.id)
                    && Objects.equals(timestamp, that.timestamp)
                    && Objects.equals(modelId, that.modelId);
            }

            @Override
            public int hashCode() { return Objects.hash(id, timestamp, modelId); }
        }

        public static class Source extends StreamPart {
            @JsonProperty("id") private String id = "";
            @JsonProperty("source_type") private String sourceType = "";
            @JsonProperty("url") private String url;
            @JsonProperty("title") private String title;
            @JsonProperty("provider_metadata") private JsonNode providerMetadata;

            @JsonCreator
            Source() {}

            private Source(String id, String sourceType, String url, String title, JsonNode providerMetadata) {
                this.id = id;
                this.sourceType = sourceType;
                this.url = url;
                this.title = title;
                this.providerMetadata = providerMetadata;
            }

            public String getId() { return id; }
            public String getSourceType() { return sourceType; }
            public String getUrl() { return url; }
            public String getTitle() { return title; }
            public JsonNode getProviderMetadata() { return providerMetadata; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private String id = "";
                private String sourceType = "";
                private String url;
                private String title;
                private JsonNode providerMetadata;

                public Builder id(String v) { this.id = v; return this; }
                public Builder sourceType(String v) { this.sourceType = v; return this; }
                public Builder url(String v) { this.url = v; return this; }
                public Builder title(String v) { this.title = v; return this; }
                public Builder providerMetadata(JsonNode v) { this.providerMetadata = v; return this; }

                public Source build() { return new Source(id, sourceType, url, title, providerMetadata); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Source)) return false;
                Source that = (Source) o;
                return Objects.equals(id, that.id)
                    && Objects.equals(sourceType, that.sourceType)
                    && Objects.equals(url, that.url)
                    && Objects.equals(title, that.title)
                    && Objects.equals(providerMetadata, that.providerMetadata);
            }

            @Override
            public int hashCode() { return Objects.hash(id, sourceType, url, title, providerMetadata); }
        }

        public static class Raw extends StreamPart {
            @JsonProperty("raw_value") private JsonNode rawValue = emptyObject();

            @JsonCreator
            Raw() {}

            private Raw(JsonNode rawValue) { this.rawValue = rawValue; }

            public JsonNode getRawValue() { return rawValue; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private JsonNode rawValue = emptyObject();

                public Builder rawValue(JsonNode v) { this.rawValue = v; return this; }

                public Raw build() { return new Raw(rawValue); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Raw)) return false;
                return Objects.equals(rawValue, ((Raw) o).rawValue);
            }

            @Override
            public int hashCode() { return Objects.hash(rawValue); }
        }

        public static class Error extends StreamPart {
            @JsonProperty("error") private JsonNode error = emptyObject();

            @JsonCreator
            Error() {}

            private Error(JsonNode error) { this.error = error; }

            public JsonNode getError() { return error; }

            public static Builder builder() { return new Builder(); }

            public static class Builder {
                private JsonNode error = emptyObject();

                public Builder error(JsonNode v) { this.error = v; return this; }

                public Error build() { return new Error(error); }
            }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Error)) return false;
                return Objects.equals(error, ((Error) o).error);
            }

            @Override
            public int hashCode() { return Objects.hash(error); }
        }

        /** Fallback for variants introduced after this wrapper was written. */
        public static class Unknown extends StreamPart {
            private String tag;
            private JsonNode data;

            @JsonCreator
            Unknown() {}

            public Unknown(String tag, JsonNode data) {
                this.tag = tag;
                this.data = data;
            }

            public String getTag() { return tag; }
            public JsonNode getData() { return data; }

            @Override
            public boolean equals(Object o) {
                if (this == o) return true;
                if (!(o instanceof Unknown)) return false;
                Unknown that = (Unknown) o;
                return Objects.equals(tag, that.tag) && Objects.equals(data, that.data);
            }

            @Override
            public int hashCode() { return Objects.hash(tag, data); }
        }
    }

    /** The externally-tagged variant name for this {@link StreamPart} (e.g. "TextDelta", "ToolCall"). */
    public static String variantTag(StreamPart value) {
        if (value instanceof StreamPart.TextStart) return "TextStart";
        if (value instanceof StreamPart.TextDelta) return "TextDelta";
        if (value instanceof StreamPart.TextEnd) return "TextEnd";
        if (value instanceof StreamPart.StreamStart) return "StreamStart";
        if (value instanceof StreamPart.Finish) return "Finish";
        if (value instanceof StreamPart.ToolInputStart) return "ToolInputStart";
        if (value instanceof StreamPart.ToolInputDelta) return "ToolInputDelta";
        if (value instanceof StreamPart.ToolInputEnd) return "ToolInputEnd";
        if (value instanceof StreamPart.ToolCall) return "ToolCall";
        if (value instanceof StreamPart.ToolResult) return "ToolResult";
        if (value instanceof StreamPart.File) return "File";
        if (value instanceof StreamPart.ReasoningStart) return "ReasoningStart";
        if (value instanceof StreamPart.ReasoningDelta) return "ReasoningDelta";
        if (value instanceof StreamPart.ReasoningEnd) return "ReasoningEnd";
        if (value instanceof StreamPart.ResponseMetadata) return "ResponseMetadata";
        if (value instanceof StreamPart.Source) return "Source";
        if (value instanceof StreamPart.Raw) return "Raw";
        if (value instanceof StreamPart.Error) return "Error";
        if (value instanceof StreamPart.Unknown) return ((StreamPart.Unknown) value).getTag();
        throw new IllegalArgumentException("Unknown StreamPart: " + value);
    }

    /** Custom (de)serializer for {@link StreamPart} — externally tagged. */
    public static class StreamPartSerializer extends JsonSerializer<StreamPart> {
        @Override
        public void serialize(StreamPart value, JsonGenerator gen, SerializerProvider serializers) throws IOException {
            String tag = variantTag(value);
            if (value instanceof StreamPart.Unknown) {
                ObjectNode out = JsonNodeFactory.instance.objectNode();
                out.set(tag, ((StreamPart.Unknown) value).getData());
                gen.writeTree(out);
                return;
            }
            ObjectNode node = (ObjectNode) AimuxJson.INNER_MAPPER.valueToTree(value);
            ObjectNode out = JsonNodeFactory.instance.objectNode();
            out.set(tag, node);
            gen.writeTree(out);
        }
    }

    /** Custom (de)serializer for {@link StreamPart}. */
    public static class StreamPartDeserializer extends JsonDeserializer<StreamPart> {
        @Override
        public StreamPart deserialize(JsonParser p, DeserializationContext ctxt) throws IOException {
            JsonNode node = p.getCodec().readTree(p);
            if (!node.isObject() || node.size() != 1) {
                throw new IOException("StreamPart must be a single-key externally-tagged object, got: " + node);
            }
            String tag = node.fieldNames().next();
            JsonNode inner = node.get(tag);
            JsonNode innerObj = inner.isObject() ? inner : AimuxJson.MAPPER.createObjectNode();
            switch (tag) {
                case "TextStart": return AimuxJson.MAPPER.treeToValue(innerObj, StreamPart.TextStart.class);
                case "TextDelta": return AimuxJson.MAPPER.treeToValue(innerObj, StreamPart.TextDelta.class);
                case "TextEnd": return AimuxJson.MAPPER.treeToValue(innerObj, StreamPart.TextEnd.class);
                case "StreamStart": return AimuxJson.MAPPER.treeToValue(innerObj, StreamPart.StreamStart.class);
                case "Finish": return AimuxJson.MAPPER.treeToValue(innerObj, StreamPart.Finish.class);
                case "ToolInputStart": return AimuxJson.MAPPER.treeToValue(innerObj, StreamPart.ToolInputStart.class);
                case "ToolInputDelta": return AimuxJson.MAPPER.treeToValue(innerObj, StreamPart.ToolInputDelta.class);
                case "ToolInputEnd": return AimuxJson.MAPPER.treeToValue(innerObj, StreamPart.ToolInputEnd.class);
                case "ToolCall": return AimuxJson.MAPPER.treeToValue(innerObj, StreamPart.ToolCall.class);
                case "ToolResult": return AimuxJson.MAPPER.treeToValue(innerObj, StreamPart.ToolResult.class);
                case "File": return AimuxJson.MAPPER.treeToValue(innerObj, StreamPart.File.class);
                case "ReasoningStart": return AimuxJson.MAPPER.treeToValue(innerObj, StreamPart.ReasoningStart.class);
                case "ReasoningDelta": return AimuxJson.MAPPER.treeToValue(innerObj, StreamPart.ReasoningDelta.class);
                case "ReasoningEnd": return AimuxJson.MAPPER.treeToValue(innerObj, StreamPart.ReasoningEnd.class);
                case "ResponseMetadata": return AimuxJson.MAPPER.treeToValue(innerObj, StreamPart.ResponseMetadata.class);
                case "Source": return AimuxJson.MAPPER.treeToValue(innerObj, StreamPart.Source.class);
                case "Raw": return AimuxJson.MAPPER.treeToValue(innerObj, StreamPart.Raw.class);
                case "Error": return AimuxJson.MAPPER.treeToValue(innerObj, StreamPart.Error.class);
                default: return new StreamPart.Unknown(tag, inner);
            }
        }
    }

    private static ObjectNode emptyObject() {
        return JsonNodeFactory.instance.objectNode();
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // OpenAI Chat Completions output (RFC-0026).
    //
    // Mirrors `aimux-core::openai_output`. Field names are camelCase in Java and
    // mapped to the wire's snake_case via @JsonProperty. The `type` field is JSON
    // `"type"` (Rust `#[serde(rename = "type")]`) → `toolType`. Arbitrary-JSON
    // fields (`logprobs`, `annotations`) are JsonNode.
    // ─────────────────────────────────────────────────────────────────────────────

    /** A complete Chat Completion response (non-streaming). Mirrors OpenAI `chat.completion`. */
    public static class ChatCompletion {
        @JsonProperty("id") private String id = "";
        @JsonProperty("object") private String object = "chat.completion";
        @JsonProperty("created") private long created;
        @JsonProperty("model") private String model = "";
        @JsonProperty("choices") private List<ChatCompletionChoice> choices = new ArrayList<>();
        @JsonProperty("usage") private ChatCompletionUsage usage = new ChatCompletionUsage();
        @JsonProperty("system_fingerprint") private String systemFingerprint;

        @JsonCreator
        ChatCompletion() {}

        private ChatCompletion(String id, String object, long created, String model,
                               List<ChatCompletionChoice> choices, ChatCompletionUsage usage,
                               String systemFingerprint) {
            this.id = id; this.object = object; this.created = created; this.model = model;
            this.choices = choices; this.usage = usage; this.systemFingerprint = systemFingerprint;
        }

        public String getId() { return id; }
        public String getObject() { return object; }
        public long getCreated() { return created; }
        public String getModel() { return model; }
        public List<ChatCompletionChoice> getChoices() { return choices; }
        public ChatCompletionUsage getUsage() { return usage; }
        public String getSystemFingerprint() { return systemFingerprint; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String id = "";
            private String object = "chat.completion";
            private long created;
            private String model = "";
            private List<ChatCompletionChoice> choices = new ArrayList<>();
            private ChatCompletionUsage usage = new ChatCompletionUsage();
            private String systemFingerprint;

            public Builder id(String v) { this.id = v; return this; }
            public Builder object(String v) { this.object = v; return this; }
            public Builder created(long v) { this.created = v; return this; }
            public Builder model(String v) { this.model = v; return this; }
            public Builder choices(List<ChatCompletionChoice> v) { this.choices = v; return this; }
            public Builder usage(ChatCompletionUsage v) { this.usage = v; return this; }
            public Builder systemFingerprint(String v) { this.systemFingerprint = v; return this; }

            public ChatCompletion build() {
                return new ChatCompletion(id, object, created, model, choices, usage, systemFingerprint);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof ChatCompletion)) return false;
            ChatCompletion that = (ChatCompletion) o;
            return created == that.created
                && Objects.equals(id, that.id)
                && Objects.equals(object, that.object)
                && Objects.equals(model, that.model)
                && Objects.equals(choices, that.choices)
                && Objects.equals(usage, that.usage)
                && Objects.equals(systemFingerprint, that.systemFingerprint);
        }

        @Override
        public int hashCode() {
            return Objects.hash(id, object, created, model, choices, usage, systemFingerprint);
        }
    }

    public static class ChatCompletionChoice {
        @JsonProperty("index") private int index;
        @JsonProperty("message") private ChatCompletionMessage message = new ChatCompletionMessage();
        @JsonProperty("finish_reason") private String finishReason;
        @JsonProperty("logprobs") private JsonNode logprobs;

        @JsonCreator
        ChatCompletionChoice() {}

        private ChatCompletionChoice(int index, ChatCompletionMessage message, String finishReason, JsonNode logprobs) {
            this.index = index; this.message = message; this.finishReason = finishReason; this.logprobs = logprobs;
        }

        public int getIndex() { return index; }
        public ChatCompletionMessage getMessage() { return message; }
        public String getFinishReason() { return finishReason; }
        public JsonNode getLogprobs() { return logprobs; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private int index;
            private ChatCompletionMessage message = new ChatCompletionMessage();
            private String finishReason;
            private JsonNode logprobs;

            public Builder index(int v) { this.index = v; return this; }
            public Builder message(ChatCompletionMessage v) { this.message = v; return this; }
            public Builder finishReason(String v) { this.finishReason = v; return this; }
            public Builder logprobs(JsonNode v) { this.logprobs = v; return this; }

            public ChatCompletionChoice build() {
                return new ChatCompletionChoice(index, message, finishReason, logprobs);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof ChatCompletionChoice)) return false;
            ChatCompletionChoice that = (ChatCompletionChoice) o;
            return index == that.index
                && Objects.equals(message, that.message)
                && Objects.equals(finishReason, that.finishReason)
                && Objects.equals(logprobs, that.logprobs);
        }

        @Override
        public int hashCode() {
            return Objects.hash(index, message, finishReason, logprobs);
        }
    }

    public static class ChatCompletionMessage {
        @JsonProperty("role") private String role = "assistant";
        @JsonProperty("content") private String content;
        @JsonProperty("reasoning_content") private String reasoningContent;
        @JsonProperty("tool_calls") private List<ChatCompletionToolCall> toolCalls;
        @JsonProperty("annotations") private List<JsonNode> annotations;

        @JsonCreator
        ChatCompletionMessage() {}

        private ChatCompletionMessage(String role, String content, String reasoningContent,
                                      List<ChatCompletionToolCall> toolCalls, List<JsonNode> annotations) {
            this.role = role; this.content = content; this.reasoningContent = reasoningContent;
            this.toolCalls = toolCalls; this.annotations = annotations;
        }

        public String getRole() { return role; }
        public String getContent() { return content; }
        public String getReasoningContent() { return reasoningContent; }
        public List<ChatCompletionToolCall> getToolCalls() { return toolCalls; }
        public List<JsonNode> getAnnotations() { return annotations; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String role = "assistant";
            private String content;
            private String reasoningContent;
            private List<ChatCompletionToolCall> toolCalls;
            private List<JsonNode> annotations;

            public Builder role(String v) { this.role = v; return this; }
            public Builder content(String v) { this.content = v; return this; }
            public Builder reasoningContent(String v) { this.reasoningContent = v; return this; }
            public Builder toolCalls(List<ChatCompletionToolCall> v) { this.toolCalls = v; return this; }
            public Builder annotations(List<JsonNode> v) { this.annotations = v; return this; }

            public ChatCompletionMessage build() {
                return new ChatCompletionMessage(role, content, reasoningContent, toolCalls, annotations);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof ChatCompletionMessage)) return false;
            ChatCompletionMessage that = (ChatCompletionMessage) o;
            return Objects.equals(role, that.role)
                && Objects.equals(content, that.content)
                && Objects.equals(reasoningContent, that.reasoningContent)
                && Objects.equals(toolCalls, that.toolCalls)
                && Objects.equals(annotations, that.annotations);
        }

        @Override
        public int hashCode() {
            return Objects.hash(role, content, reasoningContent, toolCalls, annotations);
        }
    }

    /**
     * A tool call in a {@link ChatCompletionMessage}.
     *
     * <p>Wire: {@code {"id","type":"function","function":{"name","arguments"}}}. The
     * {@code type} field is JSON {@code "type"} (Rust {@code #[serde(rename = "type")]}).
     */
    public static class ChatCompletionToolCall {
        @JsonProperty("id") private String id = "";
        @JsonProperty("type") private String toolType = "function";
        @JsonProperty("function") private ChatCompletionFunction function = new ChatCompletionFunction();

        @JsonCreator
        ChatCompletionToolCall() {}

        private ChatCompletionToolCall(String id, String toolType, ChatCompletionFunction function) {
            this.id = id; this.toolType = toolType; this.function = function;
        }

        public String getId() { return id; }
        public String getToolType() { return toolType; }
        public ChatCompletionFunction getFunction() { return function; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String id = "";
            private String toolType = "function";
            private ChatCompletionFunction function = new ChatCompletionFunction();

            public Builder id(String v) { this.id = v; return this; }
            public Builder toolType(String v) { this.toolType = v; return this; }
            public Builder function(ChatCompletionFunction v) { this.function = v; return this; }

            public ChatCompletionToolCall build() { return new ChatCompletionToolCall(id, toolType, function); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof ChatCompletionToolCall)) return false;
            ChatCompletionToolCall that = (ChatCompletionToolCall) o;
            return Objects.equals(id, that.id)
                && Objects.equals(toolType, that.toolType)
                && Objects.equals(function, that.function);
        }

        @Override
        public int hashCode() {
            return Objects.hash(id, toolType, function);
        }
    }

    public static class ChatCompletionFunction {
        @JsonProperty("name") private String name = "";
        @JsonProperty("arguments") private String arguments = "";

        @JsonCreator
        ChatCompletionFunction() {}

        private ChatCompletionFunction(String name, String arguments) {
            this.name = name; this.arguments = arguments;
        }

        public String getName() { return name; }
        public String getArguments() { return arguments; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String name = "";
            private String arguments = "";

            public Builder name(String v) { this.name = v; return this; }
            public Builder arguments(String v) { this.arguments = v; return this; }

            public ChatCompletionFunction build() { return new ChatCompletionFunction(name, arguments); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof ChatCompletionFunction)) return false;
            ChatCompletionFunction that = (ChatCompletionFunction) o;
            return Objects.equals(name, that.name) && Objects.equals(arguments, that.arguments);
        }

        @Override
        public int hashCode() {
            return Objects.hash(name, arguments);
        }
    }

    /** A single Chat Completion chunk (streaming). Mirrors OpenAI `chat.completion.chunk`. */
    public static class ChatCompletionChunk {
        @JsonProperty("id") private String id = "";
        @JsonProperty("object") private String object = "chat.completion.chunk";
        @JsonProperty("created") private long created;
        @JsonProperty("model") private String model = "";
        @JsonProperty("choices") private List<ChatCompletionChunkChoice> choices = new ArrayList<>();
        @JsonProperty("usage") private ChatCompletionUsage usage;

        @JsonCreator
        ChatCompletionChunk() {}

        private ChatCompletionChunk(String id, String object, long created, String model,
                                    List<ChatCompletionChunkChoice> choices, ChatCompletionUsage usage) {
            this.id = id; this.object = object; this.created = created; this.model = model;
            this.choices = choices; this.usage = usage;
        }

        public String getId() { return id; }
        public String getObject() { return object; }
        public long getCreated() { return created; }
        public String getModel() { return model; }
        public List<ChatCompletionChunkChoice> getChoices() { return choices; }
        public ChatCompletionUsage getUsage() { return usage; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String id = "";
            private String object = "chat.completion.chunk";
            private long created;
            private String model = "";
            private List<ChatCompletionChunkChoice> choices = new ArrayList<>();
            private ChatCompletionUsage usage;

            public Builder id(String v) { this.id = v; return this; }
            public Builder object(String v) { this.object = v; return this; }
            public Builder created(long v) { this.created = v; return this; }
            public Builder model(String v) { this.model = v; return this; }
            public Builder choices(List<ChatCompletionChunkChoice> v) { this.choices = v; return this; }
            public Builder usage(ChatCompletionUsage v) { this.usage = v; return this; }

            public ChatCompletionChunk build() {
                return new ChatCompletionChunk(id, object, created, model, choices, usage);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof ChatCompletionChunk)) return false;
            ChatCompletionChunk that = (ChatCompletionChunk) o;
            return created == that.created
                && Objects.equals(id, that.id)
                && Objects.equals(object, that.object)
                && Objects.equals(model, that.model)
                && Objects.equals(choices, that.choices)
                && Objects.equals(usage, that.usage);
        }

        @Override
        public int hashCode() {
            return Objects.hash(id, object, created, model, choices, usage);
        }
    }

    public static class ChatCompletionChunkChoice {
        @JsonProperty("index") private int index;
        @JsonProperty("delta") private ChatCompletionDelta delta = new ChatCompletionDelta();
        @JsonProperty("finish_reason") private String finishReason;
        @JsonProperty("logprobs") private JsonNode logprobs;

        @JsonCreator
        ChatCompletionChunkChoice() {}

        private ChatCompletionChunkChoice(int index, ChatCompletionDelta delta, String finishReason, JsonNode logprobs) {
            this.index = index; this.delta = delta; this.finishReason = finishReason; this.logprobs = logprobs;
        }

        public int getIndex() { return index; }
        public ChatCompletionDelta getDelta() { return delta; }
        public String getFinishReason() { return finishReason; }
        public JsonNode getLogprobs() { return logprobs; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private int index;
            private ChatCompletionDelta delta = new ChatCompletionDelta();
            private String finishReason;
            private JsonNode logprobs;

            public Builder index(int v) { this.index = v; return this; }
            public Builder delta(ChatCompletionDelta v) { this.delta = v; return this; }
            public Builder finishReason(String v) { this.finishReason = v; return this; }
            public Builder logprobs(JsonNode v) { this.logprobs = v; return this; }

            public ChatCompletionChunkChoice build() {
                return new ChatCompletionChunkChoice(index, delta, finishReason, logprobs);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof ChatCompletionChunkChoice)) return false;
            ChatCompletionChunkChoice that = (ChatCompletionChunkChoice) o;
            return index == that.index
                && Objects.equals(delta, that.delta)
                && Objects.equals(finishReason, that.finishReason)
                && Objects.equals(logprobs, that.logprobs);
        }

        @Override
        public int hashCode() {
            return Objects.hash(index, delta, finishReason, logprobs);
        }
    }

    public static class ChatCompletionDelta {
        @JsonProperty("role") private String role;
        @JsonProperty("content") private String content;
        @JsonProperty("reasoning_content") private String reasoningContent;
        @JsonProperty("tool_calls") private List<ChatCompletionChunkToolCall> toolCalls;

        @JsonCreator
        ChatCompletionDelta() {}

        private ChatCompletionDelta(String role, String content, String reasoningContent,
                                    List<ChatCompletionChunkToolCall> toolCalls) {
            this.role = role; this.content = content; this.reasoningContent = reasoningContent;
            this.toolCalls = toolCalls;
        }

        public String getRole() { return role; }
        public String getContent() { return content; }
        public String getReasoningContent() { return reasoningContent; }
        public List<ChatCompletionChunkToolCall> getToolCalls() { return toolCalls; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String role;
            private String content;
            private String reasoningContent;
            private List<ChatCompletionChunkToolCall> toolCalls;

            public Builder role(String v) { this.role = v; return this; }
            public Builder content(String v) { this.content = v; return this; }
            public Builder reasoningContent(String v) { this.reasoningContent = v; return this; }
            public Builder toolCalls(List<ChatCompletionChunkToolCall> v) { this.toolCalls = v; return this; }

            public ChatCompletionDelta build() {
                return new ChatCompletionDelta(role, content, reasoningContent, toolCalls);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof ChatCompletionDelta)) return false;
            ChatCompletionDelta that = (ChatCompletionDelta) o;
            return Objects.equals(role, that.role)
                && Objects.equals(content, that.content)
                && Objects.equals(reasoningContent, that.reasoningContent)
                && Objects.equals(toolCalls, that.toolCalls);
        }

        @Override
        public int hashCode() {
            return Objects.hash(role, content, reasoningContent, toolCalls);
        }
    }

    /**
     * A tool call delta in a {@link ChatCompletionChunk}.
     *
     * <p>Wire: {@code {"index","id"?,"type":"function"?,"function":{"name"?,"arguments"?}}}.
     * The {@code type} field is JSON {@code "type"} (Rust {@code #[serde(rename = "type")]}).
     */
    public static class ChatCompletionChunkToolCall {
        @JsonProperty("index") private int index;
        @JsonProperty("id") private String id;
        @JsonProperty("type") private String toolType;
        @JsonProperty("function") private ChatCompletionChunkFunction function = new ChatCompletionChunkFunction();

        @JsonCreator
        ChatCompletionChunkToolCall() {}

        private ChatCompletionChunkToolCall(int index, String id, String toolType,
                                            ChatCompletionChunkFunction function) {
            this.index = index; this.id = id; this.toolType = toolType; this.function = function;
        }

        public int getIndex() { return index; }
        public String getId() { return id; }
        public String getToolType() { return toolType; }
        public ChatCompletionChunkFunction getFunction() { return function; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private int index;
            private String id;
            private String toolType;
            private ChatCompletionChunkFunction function = new ChatCompletionChunkFunction();

            public Builder index(int v) { this.index = v; return this; }
            public Builder id(String v) { this.id = v; return this; }
            public Builder toolType(String v) { this.toolType = v; return this; }
            public Builder function(ChatCompletionChunkFunction v) { this.function = v; return this; }

            public ChatCompletionChunkToolCall build() {
                return new ChatCompletionChunkToolCall(index, id, toolType, function);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof ChatCompletionChunkToolCall)) return false;
            ChatCompletionChunkToolCall that = (ChatCompletionChunkToolCall) o;
            return index == that.index
                && Objects.equals(id, that.id)
                && Objects.equals(toolType, that.toolType)
                && Objects.equals(function, that.function);
        }

        @Override
        public int hashCode() {
            return Objects.hash(index, id, toolType, function);
        }
    }

    public static class ChatCompletionChunkFunction {
        @JsonProperty("name") private String name;
        @JsonProperty("arguments") private String arguments;

        @JsonCreator
        ChatCompletionChunkFunction() {}

        private ChatCompletionChunkFunction(String name, String arguments) {
            this.name = name; this.arguments = arguments;
        }

        public String getName() { return name; }
        public String getArguments() { return arguments; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private String name;
            private String arguments;

            public Builder name(String v) { this.name = v; return this; }
            public Builder arguments(String v) { this.arguments = v; return this; }

            public ChatCompletionChunkFunction build() { return new ChatCompletionChunkFunction(name, arguments); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof ChatCompletionChunkFunction)) return false;
            ChatCompletionChunkFunction that = (ChatCompletionChunkFunction) o;
            return Objects.equals(name, that.name) && Objects.equals(arguments, that.arguments);
        }

        @Override
        public int hashCode() {
            return Objects.hash(name, arguments);
        }
    }

    /** Token usage statistics (shared by streaming and non-streaming). */
    public static class ChatCompletionUsage {
        @JsonProperty("prompt_tokens") private int promptTokens;
        @JsonProperty("completion_tokens") private int completionTokens;
        @JsonProperty("total_tokens") private int totalTokens;
        @JsonProperty("prompt_tokens_details") private PromptTokensDetails promptTokensDetails;
        @JsonProperty("completion_tokens_details") private CompletionTokensDetails completionTokensDetails;

        @JsonCreator
        ChatCompletionUsage() {}

        private ChatCompletionUsage(int promptTokens, int completionTokens, int totalTokens,
                                    PromptTokensDetails promptTokensDetails,
                                    CompletionTokensDetails completionTokensDetails) {
            this.promptTokens = promptTokens; this.completionTokens = completionTokens;
            this.totalTokens = totalTokens; this.promptTokensDetails = promptTokensDetails;
            this.completionTokensDetails = completionTokensDetails;
        }

        public int getPromptTokens() { return promptTokens; }
        public int getCompletionTokens() { return completionTokens; }
        public int getTotalTokens() { return totalTokens; }
        public PromptTokensDetails getPromptTokensDetails() { return promptTokensDetails; }
        public CompletionTokensDetails getCompletionTokensDetails() { return completionTokensDetails; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private int promptTokens;
            private int completionTokens;
            private int totalTokens;
            private PromptTokensDetails promptTokensDetails;
            private CompletionTokensDetails completionTokensDetails;

            public Builder promptTokens(int v) { this.promptTokens = v; return this; }
            public Builder completionTokens(int v) { this.completionTokens = v; return this; }
            public Builder totalTokens(int v) { this.totalTokens = v; return this; }
            public Builder promptTokensDetails(PromptTokensDetails v) { this.promptTokensDetails = v; return this; }
            public Builder completionTokensDetails(CompletionTokensDetails v) { this.completionTokensDetails = v; return this; }

            public ChatCompletionUsage build() {
                return new ChatCompletionUsage(promptTokens, completionTokens, totalTokens,
                    promptTokensDetails, completionTokensDetails);
            }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof ChatCompletionUsage)) return false;
            ChatCompletionUsage that = (ChatCompletionUsage) o;
            return promptTokens == that.promptTokens
                && completionTokens == that.completionTokens
                && totalTokens == that.totalTokens
                && Objects.equals(promptTokensDetails, that.promptTokensDetails)
                && Objects.equals(completionTokensDetails, that.completionTokensDetails);
        }

        @Override
        public int hashCode() {
            return Objects.hash(promptTokens, completionTokens, totalTokens,
                promptTokensDetails, completionTokensDetails);
        }
    }

    public static class PromptTokensDetails {
        @JsonProperty("cached_tokens") private int cachedTokens;
        @JsonProperty("cache_write_tokens") private Integer cacheWriteTokens;

        @JsonCreator
        PromptTokensDetails() {}

        private PromptTokensDetails(int cachedTokens, Integer cacheWriteTokens) {
            this.cachedTokens = cachedTokens; this.cacheWriteTokens = cacheWriteTokens;
        }

        public int getCachedTokens() { return cachedTokens; }
        public Integer getCacheWriteTokens() { return cacheWriteTokens; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private int cachedTokens;
            private Integer cacheWriteTokens;

            public Builder cachedTokens(int v) { this.cachedTokens = v; return this; }
            public Builder cacheWriteTokens(Integer v) { this.cacheWriteTokens = v; return this; }

            public PromptTokensDetails build() { return new PromptTokensDetails(cachedTokens, cacheWriteTokens); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof PromptTokensDetails)) return false;
            PromptTokensDetails that = (PromptTokensDetails) o;
            return cachedTokens == that.cachedTokens
                && Objects.equals(cacheWriteTokens, that.cacheWriteTokens);
        }

        @Override
        public int hashCode() {
            return Objects.hash(cachedTokens, cacheWriteTokens);
        }
    }

    public static class CompletionTokensDetails {
        @JsonProperty("reasoning_tokens") private Integer reasoningTokens;

        @JsonCreator
        CompletionTokensDetails() {}

        private CompletionTokensDetails(Integer reasoningTokens) {
            this.reasoningTokens = reasoningTokens;
        }

        public Integer getReasoningTokens() { return reasoningTokens; }

        public static Builder builder() { return new Builder(); }

        public static class Builder {
            private Integer reasoningTokens;

            public Builder reasoningTokens(Integer v) { this.reasoningTokens = v; return this; }

            public CompletionTokensDetails build() { return new CompletionTokensDetails(reasoningTokens); }
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof CompletionTokensDetails)) return false;
            CompletionTokensDetails that = (CompletionTokensDetails) o;
            return Objects.equals(reasoningTokens, that.reasoningTokens);
        }

        @Override
        public int hashCode() {
            return Objects.hash(reasoningTokens);
        }
    }
}
