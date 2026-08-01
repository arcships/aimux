package io.aimux;

import com.fasterxml.jackson.databind.ObjectMapper;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.util.Arrays;
import java.util.List;

import static org.assertj.core.api.Assertions.assertThat;
import static org.assertj.core.api.Assertions.assertThatThrownBy;

/**
 * Round-trip serialization tests for the typed multimodal types in
 * {@link MultimodalTypes}.
 *
 * <p>Each modality's result is decoded from the same canned wire-format JSON
 * used by the Go binding's {@code Parse*Result} tests (see
 * {@code bindings/go/multimodal_test.go}), its key fields are asserted, then it
 * is re-serialized and decoded again to prove the round-trip is lossless. The
 * re-serialized wire form is also pinned against the expected normalized JSON
 * (the mapper uses {@code NON_NULL} inclusion: only {@code null} fields are
 * omitted on encode, so empty lists, empty strings, zero primitives and zero
 * boxed numbers are all emitted; nothing is lost because the decode defaults
 * match).
 *
 * <p>Special attention is paid to the three hand-written
 * {@code JsonSerializer}/{@code JsonDeserializer} pairs —
 * {@link MultimodalTypes.AudioData} (Base64/Binary),
 * {@link MultimodalTypes.ImageOutputs} (Base64/Binary) and
 * {@link MultimodalTypes.VideoData} (Url/Base64/Binary) — which implement the
 * serde-style externally-tagged enum wire format ({@code {"Base64": ...}}) and
 * are the most bug-prone part of the file. Dedicated tests below exercise every
 * variant of each union.
 *
 * <p>These are pure serialization tests: no native library is loaded — the
 * {@link MultimodalTypes.AimuxJson} mapper is configured in Java and round-trips
 * JSON without touching the FFI.
 */
class MultimodalTypesTest {

    private static final ObjectMapper M = MultimodalTypes.AimuxJson.MAPPER;

    /** Semantic JSON equality — parses both strings and compares the trees. */
    private static boolean jsonEquals(String a, String b) throws IOException {
        return M.readTree(a).equals(M.readTree(b));
    }

    // ── Embedding ──────────────────────────────────────────────────────────

    @Test
    void embeddingResultRoundTrip() throws Exception {
        // Same shape as Go ParseEmbeddingResult.
        String json = "{\"embeddings\":[[0.1,0.2,0.3],[0.4,0.5,0.6]],"
            + "\"usage\":{\"tokens\":5},\"warnings\":[]}";

        MultimodalTypes.EmbeddingResult result =
            M.readValue(json, MultimodalTypes.EmbeddingResult.class);

        // Decode (mirrors Go: 2 embeddings, 3 dims each, usage.tokens == 5).
        assertThat(result.getEmbeddings()).hasSize(2);
        assertThat(result.getEmbeddings().get(0)).hasSize(3);
        assertThat(result.getUsage()).isNotNull();
        assertThat(result.getUsage().getTokens()).isEqualTo(5L);

        // Round-trip: serialize → deserialize → equals (lossless).
        String out = M.writeValueAsString(result);
        MultimodalTypes.EmbeddingResult rt =
            M.readValue(out, MultimodalTypes.EmbeddingResult.class);
        assertThat(rt).isEqualTo(result);

        // Wire form: NON_NULL keeps the empty `warnings` list.
        assertThat(M.readTree(out)).as("wire form: %s", out).isEqualTo(M.readTree(
            "{\"embeddings\":[[0.1,0.2,0.3],[0.4,0.5,0.6]],\"usage\":{\"tokens\":5},\"warnings\":[]}"));
    }

    // ── Speech (TTS) ───────────────────────────────────────────────────────

    @Test
    void speechResultRoundTrip() throws Exception {
        // Same shape as Go ParseSpeechResult — exercises AudioData.Base64.
        String json = "{\"audio\":{\"Base64\":\"aGVsbG8=\"},"
            + "\"warnings\":[],\"response\":{\"id\":\"resp-1\"}}";

        MultimodalTypes.SpeechResult result =
            M.readValue(json, MultimodalTypes.SpeechResult.class);

        // Decode (mirrors Go: audio is the Base64 variant, value "aGVsbG8=").
        assertThat(result.getAudio()).isInstanceOf(MultimodalTypes.AudioData.Base64.class);
        assertThat(((MultimodalTypes.AudioData.Base64) result.getAudio()).getValue())
            .isEqualTo("aGVsbG8=");

        String out = M.writeValueAsString(result);
        MultimodalTypes.SpeechResult rt =
            M.readValue(out, MultimodalTypes.SpeechResult.class);
        assertThat(rt).isEqualTo(result);

        // The custom AudioData serializer re-emits the {"Base64": "..."} tag.
        assertThat(M.readTree(out)).as("wire form: %s", out).isEqualTo(M.readTree(
            "{\"audio\":{\"Base64\":\"aGVsbG8=\"},\"warnings\":[],\"response\":{}}"));
    }

    // ── Image ──────────────────────────────────────────────────────────────

    @Test
    void imageResultRoundTrip() throws Exception {
        // Same shape as Go ParseImageResult — exercises ImageOutputs.Base64.
        String json = "{\"images\":{\"Base64\":[\"aW1hZ2Ux\"]},"
            + "\"warnings\":[],\"response\":{\"id\":\"resp-1\"}}";

        MultimodalTypes.ImageResult result =
            M.readValue(json, MultimodalTypes.ImageResult.class);

        // Decode (mirrors Go: 1 base64 image, "aW1hZ2Ux").
        assertThat(result.getImages()).isInstanceOf(MultimodalTypes.ImageOutputs.Base64.class);
        List<String> b64 =
            ((MultimodalTypes.ImageOutputs.Base64) result.getImages()).getValue();
        assertThat(b64).hasSize(1);
        assertThat(b64.get(0)).isEqualTo("aW1hZ2Ux");

        String out = M.writeValueAsString(result);
        MultimodalTypes.ImageResult rt =
            M.readValue(out, MultimodalTypes.ImageResult.class);
        assertThat(rt).isEqualTo(result);

        // The custom ImageOutputs serializer re-emits the {"Base64": [...]} tag.
        assertThat(M.readTree(out)).as("wire form: %s", out).isEqualTo(M.readTree(
            "{\"images\":{\"Base64\":[\"aW1hZ2Ux\"]},\"warnings\":[],\"response\":{}}"));
    }

    // ── Transcription (STT) ────────────────────────────────────────────────

    @Test
    void transcriptionResultRoundTrip() throws Exception {
        // Same shape as Go ParseTranscriptionResult.
        String json = "{\"text\":\"Hello world\","
            + "\"segments\":[{\"text\":\"Hello\",\"start\":0.0,\"end\":1.0}],"
            + "\"language\":\"en\",\"warnings\":[],\"response\":{\"id\":\"resp-1\"}}";

        MultimodalTypes.TranscriptionResult result =
            M.readValue(json, MultimodalTypes.TranscriptionResult.class);

        // Decode (mirrors Go: text, 1 segment, segment text, language).
        assertThat(result.getText()).isEqualTo("Hello world");
        assertThat(result.getSegments()).hasSize(1);
        assertThat(result.getSegments().get(0).getText()).isEqualTo("Hello");
        assertThat(result.getSegments().get(0).getStart()).isEqualTo(0.0);
        assertThat(result.getSegments().get(0).getEnd()).isEqualTo(1.0);
        assertThat(result.getLanguage()).isEqualTo("en");

        String out = M.writeValueAsString(result);
        MultimodalTypes.TranscriptionResult rt =
            M.readValue(out, MultimodalTypes.TranscriptionResult.class);

        // Full round-trip is lossless: NON_NULL keeps `start:0.0` (the field's
        // declared default is null, not 0.0, so a set 0.0 is NOT omitted).
        assertThat(rt).isEqualTo(result);
        assertThat(rt.getSegments().get(0).getStart()).isEqualTo(0.0);
        assertThat(rt.getSegments().get(0).getEnd()).isEqualTo(1.0);
        assertThat(rt.getText()).isEqualTo("Hello world");
        assertThat(rt.getLanguage()).isEqualTo("en");

        // Wire form retains start:0.0 and the empty `warnings` list (NON_NULL
        // suppresses only null, not zero values or empty collections).
        assertThat(M.readTree(out)).as("wire form: %s", out).isEqualTo(M.readTree(
            "{\"text\":\"Hello world\",\"segments\":[{\"text\":\"Hello\",\"start\":0.0,\"end\":1.0}],"
                + "\"language\":\"en\",\"warnings\":[],\"response\":{}}"));
    }

    // ── Reranking ──────────────────────────────────────────────────────────

    @Test
    void rerankingResultRoundTrip() throws Exception {
        // Same shape as Go ParseRerankingResult.
        String json = "{\"ranking\":["
            + "{\"index\":1,\"relevance_score\":0.95},"
            + "{\"index\":0,\"relevance_score\":0.30}"
            + "],\"warnings\":[]}";

        MultimodalTypes.RerankingResult result =
            M.readValue(json, MultimodalTypes.RerankingResult.class);

        // Decode (mirrors Go: 2 ranks, first index 1 / score 0.95).
        assertThat(result.getRanking()).hasSize(2);
        assertThat(result.getRanking().get(0).getIndex()).isEqualTo(1);
        assertThat(result.getRanking().get(0).getRelevanceScore()).isEqualTo(0.95);
        assertThat(result.getRanking().get(1).getIndex()).isEqualTo(0);
        assertThat(result.getRanking().get(1).getRelevanceScore()).isEqualTo(0.30);

        String out = M.writeValueAsString(result);
        MultimodalTypes.RerankingResult rt =
            M.readValue(out, MultimodalTypes.RerankingResult.class);
        assertThat(rt).isEqualTo(result);

        // NON_NULL keeps the primitive `index:0` on the second rank (a primitive
        // int is never null, so it is not suppressed) and the empty `warnings`
        // list; it round-trips losslessly either way.
        assertThat(M.readTree(out)).as("wire form: %s", out).isEqualTo(M.readTree(
            "{\"ranking\":[{\"index\":1,\"relevance_score\":0.95},"
            + "{\"index\":0,\"relevance_score\":0.3}],\"warnings\":[]}"));
    }

    // ── Video ──────────────────────────────────────────────────────────────

    @Test
    void videoResultRoundTrip() throws Exception {
        // Same shape as Go ParseVideoResult — exercises VideoData.Url.
        String json = "{\"videos\":[{\"Url\":"
            + "{\"url\":\"https://example.com/video.mp4\",\"media_type\":\"video/mp4\"}}],"
            + "\"warnings\":[],\"response\":{\"id\":\"resp-1\"}}";

        MultimodalTypes.VideoResult result =
            M.readValue(json, MultimodalTypes.VideoResult.class);

        // Decode (mirrors Go: 1 video, Url variant, url string).
        assertThat(result.getVideos()).hasSize(1);
        assertThat(result.getVideos().get(0)).isInstanceOf(MultimodalTypes.VideoData.Url.class);
        MultimodalTypes.VideoUrlData url =
            ((MultimodalTypes.VideoData.Url) result.getVideos().get(0)).getValue();
        assertThat(url.getUrl()).isEqualTo("https://example.com/video.mp4");
        assertThat(url.getMediaType()).isEqualTo("video/mp4");

        String out = M.writeValueAsString(result);
        MultimodalTypes.VideoResult rt =
            M.readValue(out, MultimodalTypes.VideoResult.class);
        assertThat(rt).isEqualTo(result);

        // The custom VideoData serializer re-emits the {"Url": {...}} tag.
        assertThat(M.readTree(out)).as("wire form: %s", out).isEqualTo(M.readTree(
            "{\"videos\":[{\"Url\":{\"url\":\"https://example.com/video.mp4\","
            + "\"media_type\":\"video/mp4\"}}],\"warnings\":[],\"response\":{}}"));
    }

    // ── Search ─────────────────────────────────────────────────────────────

    @Test
    void searchResultRoundTrip() throws Exception {
        // Same shape as Go ParseSearchResult.
        String json = "{\"results\":[{\"title\":\"Rust\","
            + "\"url\":\"https://rust-lang.org\",\"content\":\"Rust is...\"}],"
            + "\"answer\":\"Rust is a systems language.\",\"warnings\":[]}";

        MultimodalTypes.SearchResult result =
            M.readValue(json, MultimodalTypes.SearchResult.class);

        // Decode (mirrors Go: 1 result, title "Rust", answer string).
        assertThat(result.getResults()).hasSize(1);
        assertThat(result.getResults().get(0).getTitle()).isEqualTo("Rust");
        assertThat(result.getResults().get(0).getUrl()).isEqualTo("https://rust-lang.org");
        assertThat(result.getAnswer()).isEqualTo("Rust is a systems language.");

        String out = M.writeValueAsString(result);
        MultimodalTypes.SearchResult rt =
            M.readValue(out, MultimodalTypes.SearchResult.class);
        assertThat(rt).isEqualTo(result);

        assertThat(M.readTree(out)).as("wire form: %s", out).isEqualTo(M.readTree(
            "{\"results\":[{\"title\":\"Rust\",\"url\":\"https://rust-lang.org\","
            + "\"content\":\"Rust is...\"}],\"answer\":\"Rust is a systems language.\",\"warnings\":[]}"));
    }

    // ── Files (upload) ─────────────────────────────────────────────────────

    @Test
    void uploadFileResultRoundTrip() throws Exception {
        // Same shape as Go ParseUploadFileResult.
        String json = "{\"provider_reference\":{\"openai\":\"file-abc123\"},"
            + "\"media_type\":\"application/pdf\",\"filename\":\"doc.pdf\",\"warnings\":[]}";

        MultimodalTypes.UploadFileResult result =
            M.readValue(json, MultimodalTypes.UploadFileResult.class);

        // Decode (mirrors Go: provider_reference.openai, media_type).
        assertThat(result.getProviderReference()).containsEntry("openai", "file-abc123");
        assertThat(result.getMediaType()).isEqualTo("application/pdf");
        assertThat(result.getFilename()).isEqualTo("doc.pdf");

        String out = M.writeValueAsString(result);
        MultimodalTypes.UploadFileResult rt =
            M.readValue(out, MultimodalTypes.UploadFileResult.class);
        assertThat(rt).isEqualTo(result);

        assertThat(M.readTree(out)).as("wire form: %s", out).isEqualTo(M.readTree(
            "{\"provider_reference\":{\"openai\":\"file-abc123\"},"
            + "\"media_type\":\"application/pdf\",\"filename\":\"doc.pdf\",\"warnings\":[]}"));
    }

    // ── Custom serializer focus: the three sealed unions ───────────────────
    // These directly exercise the hand-written JsonSerializer/JsonDeserializer
    // pairs across every wire variant — the area most likely to harbor bugs.

    @Test
    void audioDataSerializerAllVariants() throws Exception {
        // Base64 variant.
        MultimodalTypes.AudioData base64 = new MultimodalTypes.AudioData.Base64("aGVsbG8=");
        String base64Json = M.writeValueAsString(base64);
        assertThat(base64Json).isEqualTo("{\"Base64\":\"aGVsbG8=\"}");
        MultimodalTypes.AudioData base64Back =
            M.readValue(base64Json, MultimodalTypes.AudioData.class);
        assertThat(base64Back).isInstanceOf(MultimodalTypes.AudioData.Base64.class);
        assertThat(((MultimodalTypes.AudioData.Base64) base64Back).getValue())
            .isEqualTo("aGVsbG8=");

        // Binary variant (each element a 0–255 byte value).
        MultimodalTypes.AudioData binary =
            new MultimodalTypes.AudioData.Binary(Arrays.asList(72, 101, 108, 108, 111));
        String binaryJson = M.writeValueAsString(binary);
        assertThat(binaryJson).isEqualTo("{\"Binary\":[72,101,108,108,111]}");
        MultimodalTypes.AudioData binaryBack =
            M.readValue(binaryJson, MultimodalTypes.AudioData.class);
        assertThat(binaryBack).isInstanceOf(MultimodalTypes.AudioData.Binary.class);
        assertThat(((MultimodalTypes.AudioData.Binary) binaryBack).getValue())
            .isEqualTo(Arrays.asList(72, 101, 108, 108, 111));

        // Unknown tag → the deserializer rejects it.
        assertThatThrownBy(() -> M.readValue("{\"Quux\":\"x\"}", MultimodalTypes.AudioData.class))
            .isInstanceOf(IOException.class);
    }

    @Test
    void imageOutputsSerializerAllVariants() throws Exception {
        // Base64 variant (list of strings).
        MultimodalTypes.ImageOutputs base64 =
            new MultimodalTypes.ImageOutputs.Base64(Arrays.asList("aW1hZ2Ux", "aW1hZ2Uy"));
        String base64Json = M.writeValueAsString(base64);
        assertThat(base64Json).isEqualTo("{\"Base64\":[\"aW1hZ2Ux\",\"aW1hZ2Uy\"]}");
        MultimodalTypes.ImageOutputs base64Back =
            M.readValue(base64Json, MultimodalTypes.ImageOutputs.class);
        assertThat(base64Back).isInstanceOf(MultimodalTypes.ImageOutputs.Base64.class);
        assertThat(((MultimodalTypes.ImageOutputs.Base64) base64Back).getValue())
            .isEqualTo(Arrays.asList("aW1hZ2Ux", "aW1hZ2Uy"));

        // Binary variant (list of byte arrays).
        MultimodalTypes.ImageOutputs binary = new MultimodalTypes.ImageOutputs.Binary(
            Arrays.asList(Arrays.asList(1, 2, 3), Arrays.asList(4, 5, 6)));
        String binaryJson = M.writeValueAsString(binary);
        assertThat(binaryJson).isEqualTo("{\"Binary\":[[1,2,3],[4,5,6]]}");
        MultimodalTypes.ImageOutputs binaryBack =
            M.readValue(binaryJson, MultimodalTypes.ImageOutputs.class);
        assertThat(binaryBack).isInstanceOf(MultimodalTypes.ImageOutputs.Binary.class);
        assertThat(((MultimodalTypes.ImageOutputs.Binary) binaryBack).getValue())
            .isEqualTo(Arrays.asList(Arrays.asList(1, 2, 3), Arrays.asList(4, 5, 6)));

        assertThatThrownBy(() -> M.readValue("{\"Quux\":[1]}", MultimodalTypes.ImageOutputs.class))
            .isInstanceOf(IOException.class);
    }

    @Test
    void videoDataSerializerAllVariants() throws Exception {
        // Url variant.
        MultimodalTypes.VideoData url = new MultimodalTypes.VideoData.Url(
            MultimodalTypes.VideoUrlData.of("https://example.com/v.mp4", "video/mp4"));
        String urlJson = M.writeValueAsString(url);
        assertThat(urlJson).isEqualTo(
            "{\"Url\":{\"url\":\"https://example.com/v.mp4\",\"media_type\":\"video/mp4\"}}");
        MultimodalTypes.VideoData urlBack = M.readValue(urlJson, MultimodalTypes.VideoData.class);
        assertThat(urlBack).isInstanceOf(MultimodalTypes.VideoData.Url.class);
        assertThat(((MultimodalTypes.VideoData.Url) urlBack).getValue().getUrl())
            .isEqualTo("https://example.com/v.mp4");

        // Base64 variant.
        MultimodalTypes.VideoData base64 = new MultimodalTypes.VideoData.Base64(
            MultimodalTypes.VideoBase64Data.of("AAAA", "video/mp4"));
        String base64Json = M.writeValueAsString(base64);
        assertThat(base64Json).isEqualTo(
            "{\"Base64\":{\"data\":\"AAAA\",\"media_type\":\"video/mp4\"}}");
        MultimodalTypes.VideoData base64Back =
            M.readValue(base64Json, MultimodalTypes.VideoData.class);
        assertThat(base64Back).isInstanceOf(MultimodalTypes.VideoData.Base64.class);
        assertThat(((MultimodalTypes.VideoData.Base64) base64Back).getValue().getData())
            .isEqualTo("AAAA");

        // Binary variant.
        MultimodalTypes.VideoData binary = new MultimodalTypes.VideoData.Binary(
            MultimodalTypes.VideoBinaryData.of(Arrays.asList(1, 2, 3), "video/mp4"));
        String binaryJson = M.writeValueAsString(binary);
        assertThat(binaryJson).isEqualTo(
            "{\"Binary\":{\"data\":[1,2,3],\"media_type\":\"video/mp4\"}}");
        MultimodalTypes.VideoData binaryBack =
            M.readValue(binaryJson, MultimodalTypes.VideoData.class);
        assertThat(binaryBack).isInstanceOf(MultimodalTypes.VideoData.Binary.class);
        assertThat(((MultimodalTypes.VideoData.Binary) binaryBack).getValue().getData())
            .isEqualTo(Arrays.asList(1, 2, 3));

        assertThatThrownBy(() -> M.readValue("{\"Quux\":{}}", MultimodalTypes.VideoData.class))
            .isInstanceOf(IOException.class);
    }
}
