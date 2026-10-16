package ai.arcships.aimux;

import ai.arcships.aimux.types.*;
import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import java.io.IOException;
import java.math.BigInteger;
import java.util.Arrays;
import java.util.Base64;
import org.junit.jupiter.api.Test;
import static org.assertj.core.api.Assertions.*;

/** Modality and codec contracts exercised through generated DTOs and Jackson. */
class MultimodalTypesTest {
    private static final ObjectMapper M = DtoJson.MAPPER;

    private static <T> T roundTrip(String json, Class<T> type) throws IOException {
        T value = M.readValue(json, type);
        String encoded = M.writeValueAsString(value);
        assertThat(M.readTree(encoded).equals((left, right) -> left.isNumber() && right.isNumber()
            ? left.decimalValue().compareTo(right.decimalValue()) : (left.equals(right) ? 0 : 1), M.readTree(json))).isTrue();
        assertThat(M.<JsonNode>valueToTree(M.readValue(encoded, type))).isEqualTo(M.readTree(encoded));
        return value;
    }

    private static void assertSameJson(Object actual, Object expected) {
        assertThat(M.<JsonNode>valueToTree(actual)).isEqualTo(M.valueToTree(expected));
    }

    @Test void embeddingResultRoundTrip() throws Exception {
        EmbeddingResult result = roundTrip("{\"embeddings\":[[0.1,0.2,0.3],[0.4,0.5,0.6]],\"usage\":{\"tokens\":5},\"warnings\":[]}", EmbeddingResult.class);
        assertThat(result.getEmbeddings()).hasSize(2);
        assertThat(result.getEmbeddings().get(0)).hasSize(3);
        assertThat(result.getUsage().getTokens()).isEqualTo(5L);
    }

    @Test void speechResultRoundTrip() throws Exception {
        SpeechResult result = roundTrip("{\"audio\":\"aGVsbG8=\",\"warnings\":[],\"usage\":null,\"response\":{}}", SpeechResult.class);
        assertThat(result.getAudio().getValue()).isEqualTo("aGVsbG8=");
    }

    @Test void imageResultRoundTrip() throws Exception {
        ImageResult result = roundTrip("{\"images\":[\"aW1hZ2Ux\"],\"warnings\":[],\"response\":{}}", ImageResult.class);
        assertThat(result.getImages().getValue()).containsExactly("aW1hZ2Ux");
    }

    @Test void transcriptionResultRoundTrip() throws Exception {
        TranscriptionResult result = roundTrip("{\"text\":\"Hello world\",\"segments\":[{\"text\":\"Hello\",\"startSecond\":0.0,\"endSecond\":1.0}],\"language\":\"en\",\"warnings\":[],\"response\":{}}", TranscriptionResult.class);
        assertThat(result.getText()).isEqualTo("Hello world");
        assertThat(result.getSegments().get(0).getStartSecond()).isEqualByComparingTo("0");
        assertThat(result.getSegments().get(0).getEndSecond()).isEqualByComparingTo("1");
        assertThat(result.getLanguage()).isEqualTo("en");
    }

    @Test void rerankingResultRoundTrip() throws Exception {
        RerankingResult result = roundTrip("{\"ranking\":[{\"index\":1,\"relevanceScore\":0.95},{\"index\":0,\"relevanceScore\":0.30}],\"warnings\":[]}", RerankingResult.class);
        assertThat(result.getRanking().get(0).getIndex()).isEqualTo(1L);
        assertThat(result.getRanking().get(0).getRelevanceScore()).isEqualByComparingTo("0.95");
        assertThat(result.getRanking().get(1).getIndex()).isEqualTo(0L);
    }

    @Test void videoResultRoundTrip() throws Exception {
        VideoResult result = roundTrip("{\"videos\":[{\"type\":\"url\",\"url\":\"https://example.com/video.mp4\",\"mediaType\":\"video/mp4\"}],\"warnings\":[],\"response\":{}}", VideoResult.class);
        assertThat(result.getVideos().get(0)).isInstanceOf(VideoData.Url.class);
        assertThat(((VideoData.Url) result.getVideos().get(0)).getUrl()).isEqualTo("https://example.com/video.mp4");
    }

    @Test void videoPollOptionsRoundTrip() throws Exception {
        VideoCallOptions options = VideoCallOptions.builder().prompt("a cat")
            .poll(VideoPollOptions.builder().intervalMs(1000L).timeoutMs(120000L).build()).build();
        assertSameJson(roundTrip(DtoJson.stringify(options), VideoCallOptions.class), options);
        assertThat(options.getPoll().getIntervalMs()).isEqualTo(1000L);
    }

    @Test void videoFpsIsEmittedAsAnIntegerForRustU32() throws Exception {
        VideoCallOptions options = VideoCallOptions.builder().prompt("a cat").fps(24L).build();
        assertThat(M.readTree(DtoJson.stringify(options)).get("fps").isIntegralNumber()).isTrue();
        assertThatThrownBy(() -> M.readValue("{\"prompt\":\"a cat\",\"fps\":24.5}", VideoCallOptions.class)).isInstanceOf(IOException.class);
    }

    @Test void searchResultRoundTrip() throws Exception {
        SearchResult result = roundTrip("{\"results\":[{\"title\":\"Rust\",\"url\":\"https://rust-lang.org\",\"content\":\"Rust is...\"}],\"answer\":\"Rust is a systems language.\",\"warnings\":[]}", SearchResult.class);
        assertThat(result.getResults().get(0).getTitle()).isEqualTo("Rust");
        assertThat(result.getAnswer()).isEqualTo("Rust is a systems language.");
    }

    @Test void uploadFileResultRoundTrip() throws Exception {
        UploadFileResult result = roundTrip("{\"providerReference\":{\"openai\":\"file-abc123\"},\"mediaType\":\"application/pdf\",\"filename\":\"doc.pdf\",\"warnings\":[]}", UploadFileResult.class);
        assertThat(result.getProviderReference()).containsEntry("openai", "file-abc123");
        assertThat(result.getMediaType()).isEqualTo("application/pdf");
        assertThat(result.getFilename()).isEqualTo("doc.pdf");
    }

    @Test void audioDataSerializerAllVariants() throws Exception {
        AudioData data = new AudioData("aGVsbG8=");
        assertSameJson(roundTrip("\"aGVsbG8=\"", AudioData.class), data);
        String binary = Base64.getEncoder().encodeToString(new byte[] {0, 1, 2, (byte) 250, (byte) 255});
        assertThat(DtoJson.stringify(new AudioData(binary))).isEqualTo("\"AAEC+v8=\"");
        assertThat(DtoJson.bytes(binary)).containsExactly(0, 1, 2, (byte) 250, (byte) 255);
        assertThatThrownBy(() -> M.readValue("[72,101]", AudioData.class)).isInstanceOf(IOException.class);
        assertThatThrownBy(() -> M.readValue("{}", AudioData.class)).isInstanceOf(IOException.class);
    }

    @Test void promptAndFileBytesAreBase64Strings() throws Exception {
        UserContentPart image = UserContentPart.Image.builder().image("iVBORw==").mediaType("image/png").build();
        assertSameJson(roundTrip("{\"type\":\"image\",\"image\":\"iVBORw==\",\"mediaType\":\"image/png\"}", UserContentPart.class), image);
        assertThatThrownBy(() -> M.readValue("{\"type\":\"image\",\"image\":[1,2],\"mediaType\":\"image/png\"}", UserContentPart.class)).isInstanceOf(IOException.class);
        FilePartData data = new FilePartData.Branch1(FileData.Data.builder().data(new FileBytes("AAEC+v8=")).build());
        UserContentPart file = UserContentPart.File.builder().data(data).mediaType("application/pdf").build();
        roundTrip(DtoJson.stringify(file), UserContentPart.class);
        assertThat(roundTrip("\"aGk=\"", FileBytes.class).getValue()).isEqualTo("aGk=");
        assertThatThrownBy(() -> M.readValue("[104,105]", FileBytes.class)).isInstanceOf(IOException.class);
    }

    @Test void filePartsAreTheAiSdkFilePart() throws Exception {
        String[] data = {
            "{\"type\":\"data\",\"data\":\"aGk=\"}",
            "{\"type\":\"url\",\"url\":\"https://example.com/a.pdf\"}",
            "{\"type\":\"reference\",\"reference\":{\"openai\":\"file-abc\"}}",
            "{\"type\":\"text\",\"text\":\"hello\"}", "\"aGk=\"", "\"https://example.com/a.pdf\"", "{\"openai\":\"file-abc\"}"
        };
        for (String value : data) roundTrip("{\"type\":\"file\",\"data\":" + value + ",\"mediaType\":\"text/plain\"}", UserContentPart.class);
        assertThat(M.readValue(data[6], FilePartData.class)).isInstanceOf(FilePartData.Branch3.class);
        for (String removed : new String[] {"file-base64", "file-url", "file-reference"}) {
            assertThatThrownBy(() -> M.readValue("{\"type\":\"" + removed + "\"}", UserContentPart.class)).isInstanceOf(IOException.class).hasMessageContaining(removed);
        }
        for (String value : new String[] {"\"aGk=\"", data[1]}) {
            roundTrip("{\"type\":\"reasoning-file\",\"data\":" + value + ",\"mediaType\":\"image/png\"}", AssistantContentPart.class);
        }
    }

    @Test void generatedFilesAreBase64AndMediaType() throws Exception {
        for (String tag : new String[] {"file", "reasoning-file"}) {
            TextStreamPart part = roundTrip("{\"type\":\"" + tag + "\",\"file\":{\"base64\":\"aGk=\",\"mediaType\":\"text/plain\"},\"providerMetadata\":{\"google\":{\"thoughtSignature\":\"sig\"}}}", TextStreamPart.class);
            assertThat(DtoJson.bytes(part.getFile().getBase64().getValue())).containsExactly(104, 105);
        }
        roundTrip("{\"type\":\"file\",\"data\":{\"type\":\"url\",\"url\":\"https://example.com/a.png\"},\"mediaType\":\"image/png\"}", GenerateContent.class);
    }

    @Test void textStreamDeltasAreText() throws Exception {
        for (String tag : new String[] {"text-delta", "reasoning-delta"}) {
            TextStreamPart part = roundTrip("{\"type\":\"" + tag + "\",\"id\":\"t1\",\"text\":\"Hello\"}", TextStreamPart.class);
            assertThat(part.getText()).isEqualTo("Hello");
        }
        roundTrip("{\"type\":\"tool-input-delta\",\"id\":\"t1\",\"delta\":\"{}\"}", TextStreamPart.class);
        assertThat(TextStreamPart.ReasoningDelta.builder().id("t1").text("Hmm").build().getText()).isEqualTo("Hmm");
    }

    @Test void standaloneToolCallsAndSourcesAreTagged() throws Exception {
        String call = "{\"type\":\"tool-call\",\"toolCallId\":\"c\",\"toolName\":\"weather\",\"input\":{\"city\":\"SG\"}}";
        assertThat(roundTrip(call, TypedToolCall.class).getType()).isEqualTo("tool-call");
        assertThat(roundTrip("{\"type\":\"tool-call\",\"toolCallId\":\"c\",\"toolName\":\"t\",\"input\":\"{}\"}", TaggedRawToolCall.class).getType()).isEqualTo("tool-call");
        String url = "{\"type\":\"source\",\"sourceType\":\"url\",\"id\":\"s1\",\"url\":\"https://example.com\"}";
        roundTrip(url, TextStreamPart.class);
        roundTrip(url, ContentPart.class);
        assertThatThrownBy(() -> M.readValue("{\"toolCallId\":\"c\",\"toolName\":\"t\",\"input\":{}}", TypedToolCall.class)).isInstanceOf(IOException.class);
    }

    @Test void unknownFieldsKeepNullApartFromMissing() throws Exception {
        for (String json : new String[] {"{}", "{\"body\":null}", "{\"body\":{\"a\":null}}"}) {
            roundTrip(json, ResponseInfo.class);
            roundTrip(json, RequestInfo.class);
        }
        RequestInfo missing = RequestInfo.builder().build();
        RequestInfo explicit = RequestInfo.builder().body(M.nullNode()).build();
        assertThat(missing.hasBody()).isFalse();
        assertThat(explicit.hasBody()).isTrue();
        assertThat(explicit.getBody()).isNull();
        String approval = "{\"type\":\"tool-approval-request\",\"approvalId\":\"a1\",\"toolCallId\":\"c1\",\"inputSchemaInput\":null}";
        roundTrip(approval, AssistantContentPart.class);
    }

    @Test void ordinaryOptionalNullIsOmittedAndTypedPresenceKeepsNull() throws Exception {
        VideoCallOptions options = M.readValue("{\"prompt\":\"a cat\",\"fps\":null}", VideoCallOptions.class);
        assertThat(M.<JsonNode>valueToTree(options)).isEqualTo(M.readTree("{\"prompt\":\"a cat\"}"));
        DecisionBooleanCriteria criteria = roundTrip("{\"true\":null}", DecisionBooleanCriteria.class);
        assertThat(criteria.hasTrue_()).isTrue();
        assertThat(criteria.hasFalse_()).isFalse();
        assertThat(criteria.getTrue_()).isNull();
        assertThat(M.<JsonNode>valueToTree(DecisionBooleanCriteria.builder().false_(null).build()))
            .isEqualTo(M.readTree("{\"false\":null}"));
    }

    @Test void imageOutputsSerializerAllVariants() throws Exception {
        ImageOutputs images = new ImageOutputs(Arrays.asList("aW1hZ2Ux", "aW1hZ2Uy"));
        assertSameJson(roundTrip("[\"aW1hZ2Ux\",\"aW1hZ2Uy\"]", ImageOutputs.class), images);
        assertThatThrownBy(() -> M.readValue("[[1,2,3]]", ImageOutputs.class)).isInstanceOf(IOException.class);
        assertThatThrownBy(() -> M.readValue("{}", ImageOutputs.class)).isInstanceOf(IOException.class);
    }

    @Test void videoDataSerializerAllVariants() throws Exception {
        VideoData.Url url = VideoData.Url.builder().url("https://example.com/v.mp4").mediaType("video/mp4").build();
        assertSameJson(roundTrip(DtoJson.stringify(url), VideoData.class), url);
        VideoData.Base64 base64 = VideoData.Base64.builder().data("AAAA").mediaType("video/mp4").build();
        assertSameJson(roundTrip(DtoJson.stringify(base64), VideoData.class), base64);
        VideoData.Binary binary = VideoData.Binary.builder().data("AQID").mediaType("video/mp4").build();
        assertSameJson(roundTrip(DtoJson.stringify(binary), VideoData.class), binary);
        assertThat(DtoJson.bytes(binary.getData())).containsExactly(1, 2, 3);
        assertThatThrownBy(() -> M.readValue("{\"type\":\"quux\"}", VideoData.class)).isInstanceOf(IOException.class).hasMessageContaining("VideoData").hasMessageContaining("quux");
    }

    @Test void codecsKeepOpaqueNumbersAndFieldPresence() throws Exception {
        String json = "{\"body\":{\"integer\":18446744073709551615,\"decimal\":0.12345678901234567890123456789}}";
        RequestInfo request = roundTrip(json, RequestInfo.class);
        assertThat(request.getBody().get("integer").bigIntegerValue()).isEqualTo(new BigInteger("18446744073709551615"));
        assertThat(request.getBody().get("decimal").decimalValue()).isEqualByComparingTo("0.12345678901234567890123456789");
        ((com.fasterxml.jackson.databind.node.ObjectNode) request.getBody()).removeAll();
        assertThat(request.getBody().has("integer")).isTrue();
        assertThat(RequestInfo.builder().body(M.nullNode()).omitBody().build().hasBody()).isFalse();
    }

    @Test void generatedReadersHonorClosureAndOrderedBranches() throws Exception {
        assertThatThrownBy(() -> M.readValue("{\"unexpected\":true}", TimeoutConfiguration.class))
            .isInstanceOf(IOException.class).hasMessageContaining("TimeoutConfiguration");
        assertThat(M.<JsonNode>valueToTree(M.readValue("{\"unexpected\":true}", RequestInfo.class))).isEqualTo(M.readTree("{}"));
        assertThat(M.readValue("\"hello\"", FilePartData.class)).isInstanceOf(FilePartData.Branch2.class);
        assertThatThrownBy(() -> M.readValue("{\"type\":\"other\",\"seq\":\"0\"}", StreamFrame.class))
            .isInstanceOf(IOException.class).hasMessageContaining("StreamFrame").hasMessageContaining("other");
        assertThatThrownBy(() -> DtoJson.u64("18446744073709551616")).isInstanceOf(IllegalArgumentException.class);
        assertThatThrownBy(() -> DtoJson.u64("01")).isInstanceOf(IllegalArgumentException.class);
        assertThat(VideoCallOptions.builder().fps(4294967296L).build().getFps()).isEqualTo(4294967296L);
    }
}
