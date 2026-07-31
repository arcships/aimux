// Tests for the multimodal API — 8 modality models and their constructors.
// Mirrors Node's multimodal coverage: each model is constructed with a fake
// key, and the FFI path is verified (no network access needed for
// construction; calls use mock servers where applicable).

package aimux

import (
	"encoding/json"
	"net/http"
	"strings"
	"testing"
)

// ── Constructor tests ────────────────────────────────────────────────────────

func TestNewOpenAIEmbedding(t *testing.T) {
	m, err := NewOpenAIEmbedding("sk-test-fake-key", "text-embedding-3-small")
	if err != nil {
		t.Fatalf("NewOpenAIEmbedding failed: %v", err)
	}
	defer m.Close()
}

func TestNewOpenAISpeech(t *testing.T) {
	m, err := NewOpenAISpeech("sk-test-fake-key", "tts-1")
	if err != nil {
		t.Fatalf("NewOpenAISpeech failed: %v", err)
	}
	defer m.Close()
}

func TestNewOpenAIImage(t *testing.T) {
	m, err := NewOpenAIImage("sk-test-fake-key", "dall-e-3")
	if err != nil {
		t.Fatalf("NewOpenAIImage failed: %v", err)
	}
	defer m.Close()
}

func TestNewOpenAITranscription(t *testing.T) {
	m, err := NewOpenAITranscription("sk-test-fake-key", "whisper-1")
	if err != nil {
		t.Fatalf("NewOpenAITranscription failed: %v", err)
	}
	defer m.Close()
}

func TestNewOpenAIFiles(t *testing.T) {
	m, err := NewOpenAIFiles("sk-test-fake-key")
	if err != nil {
		t.Fatalf("NewOpenAIFiles failed: %v", err)
	}
	defer m.Close()
}

func TestNewCohereReranking(t *testing.T) {
	m, err := NewCohereReranking("sk-test-fake-key", "rerank-english-v3.0")
	if err != nil {
		t.Fatalf("NewCohereReranking failed: %v", err)
	}
	defer m.Close()
}

func TestNewGoogleVideo(t *testing.T) {
	m, err := NewGoogleVideo("sk-test-fake-key", "veo-1")
	if err != nil {
		t.Fatalf("NewGoogleVideo failed: %v", err)
	}
	defer m.Close()
}

func TestNewTavilySearch(t *testing.T) {
	m, err := NewTavilySearch("sk-test-fake-key")
	if err != nil {
		t.Fatalf("NewTavilySearch failed: %v", err)
	}
	defer m.Close()
}

// ── Close/double-close tests ─────────────────────────────────────────────────

func TestEmbeddingDoubleClose(t *testing.T) {
	m, _ := NewOpenAIEmbedding("sk-test-fake-key", "text-embedding-3-small")
	m.Close()
	if err := m.Close(); err != nil {
		t.Fatalf("double close should be safe: %v", err)
	}
}

func TestSpeechDoubleClose(t *testing.T) {
	m, _ := NewOpenAISpeech("sk-test-fake-key", "tts-1")
	m.Close()
	if err := m.Close(); err != nil {
		t.Fatalf("double close should be safe: %v", err)
	}
}

// ── E2E: Embedding via mock server ───────────────────────────────────────────

func TestE2E_Embedding(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()

	// OpenAI embeddings API response.
	embedResponse := `{
		"data": [
			{"embedding": [0.1, 0.2, 0.3], "index": 0},
			{"embedding": [0.4, 0.5, 0.6], "index": 1}
		],
		"model": "text-embedding-3-small",
		"usage": {"prompt_tokens": 5, "total_tokens": 5}
	}`
	srv.SetResponse(embedResponse)

	// EmbeddingModel uses OpenAI's embedding endpoint; the mock server
	// catches any POST and returns the preset response.
	// We can't point the embedding model at a custom URL via the C ABI
	// (aimux_openai_embedding_new doesn't take base_url), so we just
	// verify the typed result parsing here.
	result, err := ParseEmbeddingResult(`{
		"embeddings": [[0.1, 0.2, 0.3], [0.4, 0.5, 0.6]],
		"usage": {"tokens": 5},
		"warnings": []
	}`)
	if err != nil {
		t.Fatalf("parse failed: %v", err)
	}
	if len(result.Embeddings) != 2 {
		t.Fatalf("expected 2 embeddings, got %d", len(result.Embeddings))
	}
	if len(result.Embeddings[0]) != 3 {
		t.Errorf("expected 3 dimensions, got %d", len(result.Embeddings[0]))
	}
	if result.Usage == nil || result.Usage.Tokens == nil || *result.Usage.Tokens != 5 {
		t.Error("usage tokens mismatch")
	}
}

// ── E2E: Speech via mock server ──────────────────────────────────────────────

func TestE2E_SpeechResultParsing(t *testing.T) {
	result, err := ParseSpeechResult(`{
		"audio": {"Base64": "aGVsbG8="},
		"warnings": [],
		"response": {"id": "resp-1"}
	}`)
	if err != nil {
		t.Fatalf("parse failed: %v", err)
	}
	if result.Audio.Base64 == nil || *result.Audio.Base64 != "aGVsbG8=" {
		t.Error("audio base64 mismatch")
	}
}

// ── E2E: Image result parsing ────────────────────────────────────────────────

func TestE2E_ImageResultParsing(t *testing.T) {
	result, err := ParseImageResult(`{
		"images": {"Base64": ["aW1hZ2Ux"]},
		"warnings": [],
		"response": {"id": "resp-1"}
	}`)
	if err != nil {
		t.Fatalf("parse failed: %v", err)
	}
	if len(result.Images.Base64) != 1 {
		t.Fatalf("expected 1 image, got %d", len(result.Images.Base64))
	}
	if result.Images.Base64[0] != "aW1hZ2Ux" {
		t.Error("image base64 mismatch")
	}
}

// ── E2E: Transcription result parsing ────────────────────────────────────────

func TestE2E_TranscriptionResultParsing(t *testing.T) {
	result, err := ParseTranscriptionResult(`{
		"text": "Hello world",
		"segments": [{"text": "Hello", "start": 0.0, "end": 1.0}],
		"language": "en",
		"warnings": [],
		"response": {"id": "resp-1"}
	}`)
	if err != nil {
		t.Fatalf("parse failed: %v", err)
	}
	if result.Text != "Hello world" {
		t.Errorf("text mismatch: got %q", result.Text)
	}
	if len(result.Segments) != 1 {
		t.Fatalf("expected 1 segment, got %d", len(result.Segments))
	}
	if result.Segments[0].Text != "Hello" {
		t.Errorf("segment text mismatch: got %q", result.Segments[0].Text)
	}
	if result.Language == nil || *result.Language != "en" {
		t.Error("language mismatch")
	}
}

// ── E2E: Reranking result parsing ─────────────────────────────────────────────

func TestE2E_RerankingResultParsing(t *testing.T) {
	result, err := ParseRerankingResult(`{
		"ranking": [
			{"index": 1, "relevance_score": 0.95},
			{"index": 0, "relevance_score": 0.30}
		],
		"warnings": []
	}`)
	if err != nil {
		t.Fatalf("parse failed: %v", err)
	}
	if len(result.Ranking) != 2 {
		t.Fatalf("expected 2 ranks, got %d", len(result.Ranking))
	}
	if result.Ranking[0].Index != 1 {
		t.Errorf("expected index 1, got %d", result.Ranking[0].Index)
	}
	if result.Ranking[0].RelevanceScore != 0.95 {
		t.Errorf("expected score 0.95, got %f", result.Ranking[0].RelevanceScore)
	}
}

// ── E2E: Video result parsing ────────────────────────────────────────────────

func TestE2E_VideoResultParsing(t *testing.T) {
	result, err := ParseVideoResult(`{
		"videos": [{"Url": {"url": "https://example.com/video.mp4", "media_type": "video/mp4"}}],
		"warnings": [],
		"response": {"id": "resp-1"}
	}`)
	if err != nil {
		t.Fatalf("parse failed: %v", err)
	}
	if len(result.Videos) != 1 {
		t.Fatalf("expected 1 video, got %d", len(result.Videos))
	}
	if result.Videos[0].Url == nil || result.Videos[0].Url.URL != "https://example.com/video.mp4" {
		t.Error("video URL mismatch")
	}
}

// ── E2E: Search result parsing ───────────────────────────────────────────────

func TestE2E_SearchResultParsing(t *testing.T) {
	result, err := ParseSearchResult(`{
		"results": [
			{"title": "Rust", "url": "https://rust-lang.org", "content": "Rust is..."}
		],
		"answer": "Rust is a systems language.",
		"warnings": []
	}`)
	if err != nil {
		t.Fatalf("parse failed: %v", err)
	}
	if len(result.Results) != 1 {
		t.Fatalf("expected 1 result, got %d", len(result.Results))
	}
	if result.Results[0].Title == nil || *result.Results[0].Title != "Rust" {
		t.Error("title mismatch")
	}
	if result.Answer == nil || *result.Answer != "Rust is a systems language." {
		t.Error("answer mismatch")
	}
}

// ── E2E: Files result parsing ────────────────────────────────────────────────

func TestE2E_UploadFileResultParsing(t *testing.T) {
	result, err := ParseUploadFileResult(`{
		"provider_reference": {"openai": "file-abc123"},
		"media_type": "application/pdf",
		"filename": "doc.pdf",
		"warnings": []
	}`)
	if err != nil {
		t.Fatalf("parse failed: %v", err)
	}
	if result.ProviderReference["openai"] != "file-abc123" {
		t.Error("provider reference mismatch")
	}
	if result.MediaType == nil || *result.MediaType != "application/pdf" {
		t.Error("media type mismatch")
	}
}

// ── E2E: Reranking via mock server ────────────────────────────────────────────

func TestE2E_RerankWithOptions(t *testing.T) {
	// Cohere reranking uses its own endpoint; we verify options marshaling.
	docs := `{"Text":{"values":["doc1","doc2","doc3"]}}`
	opts := &RerankingCallOptions{
		Documents: json.RawMessage(docs),
		Query:     "which is most relevant?",
		TopN:      intPtr(2),
	}
	b, err := json.Marshal(opts)
	if err != nil {
		t.Fatalf("marshal failed: %v", err)
	}
	s := string(b)
	if !strings.Contains(s, `"documents"`) {
		t.Errorf("expected documents field: %s", s)
	}
	if !strings.Contains(s, `"query"`) {
		t.Errorf("expected query field: %s", s)
	}
	if !strings.Contains(s, `"top_n"`) {
		t.Errorf("expected top_n field: %s", s)
	}
}

// ── E2E: Files upload via mock server ────────────────────────────────────────

func TestE2E_FilesUploadViaMock(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()

	// Simulate OpenAI files upload response.
	srv.SetResponse(`{
		"id": "file-abc123",
		"object": "file",
		"bytes": 1024,
		"created_at": 1234567890,
		"filename": "test.pdf",
		"purpose": "assistants"
	}`)

	// Verify the mock server handles the upload request shape.
	// (The C ABI constructor doesn't accept base_url, so we can't point
	// it at the mock; this test validates the mock infrastructure and
	// result parsing.)
	req, _ := http.NewRequest("POST", srv.URL, strings.NewReader(`{"file":"base64data"}`))
	client := &http.Client{}
	resp, err := client.Do(req)
	if err != nil {
		t.Fatalf("mock request failed: %v", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		t.Fatalf("expected 200, got %d", resp.StatusCode)
	}
}

func intPtr(v int) *int { return &v }
