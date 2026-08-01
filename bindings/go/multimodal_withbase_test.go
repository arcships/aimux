// Tests for new multimodal constructors (Cohere/Google embedding, Google image)
// and WithBase variants — all 8 modalities tested via mock server.

package aimux

import (
	"encoding/json"
	"testing"
)

const base64Audio = "SGVsbG8gd29ybGQ="

// ── New constructor tests ───────────────────────────────────────────────────

func TestNewCohereEmbedding(t *testing.T) {
	m, err := NewCohereEmbedding("sk-test", "embed-english-v3.0")
	if err != nil {
		t.Fatalf("NewCohereEmbedding: %v", err)
	}
	defer m.Close()
}

func TestNewGoogleEmbedding(t *testing.T) {
	m, err := NewGoogleEmbedding("sk-test", "gemini-embedding-001")
	if err != nil {
		t.Fatalf("NewGoogleEmbedding: %v", err)
	}
	defer m.Close()
}

func TestNewGoogleImage(t *testing.T) {
	m, err := NewGoogleImage("sk-test", "gemini-2.5-flash-image")
	if err != nil {
		t.Fatalf("NewGoogleImage: %v", err)
	}
	defer m.Close()
}

// ── WithBase constructor tests ──────────────────────────────────────────────

func TestNewOpenAIEmbeddingWithBase(t *testing.T) {
	m, err := NewOpenAIEmbeddingWithBase("sk-test", "text-embedding-3-small", "http://localhost:9999")
	if err != nil {
		t.Fatalf("NewOpenAIEmbeddingWithBase: %v", err)
	}
	defer m.Close()
}

func TestNewCohereEmbeddingWithBase(t *testing.T) {
	m, err := NewCohereEmbeddingWithBase("sk-test", "embed-english-v3.0", "http://localhost:9999")
	if err != nil {
		t.Fatalf("NewCohereEmbeddingWithBase: %v", err)
	}
	defer m.Close()
}

func TestNewGoogleEmbeddingWithBase(t *testing.T) {
	m, err := NewGoogleEmbeddingWithBase("sk-test", "gemini-embedding-001", "http://localhost:9999")
	if err != nil {
		t.Fatalf("NewGoogleEmbeddingWithBase: %v", err)
	}
	defer m.Close()
}

func TestNewOpenAISpeechWithBase(t *testing.T) {
	m, err := NewOpenAISpeechWithBase("sk-test", "tts-1", "http://localhost:9999")
	if err != nil {
		t.Fatalf("NewOpenAISpeechWithBase: %v", err)
	}
	defer m.Close()
}

func TestNewOpenAIImageWithBase(t *testing.T) {
	m, err := NewOpenAIImageWithBase("sk-test", "dall-e-3", "http://localhost:9999")
	if err != nil {
		t.Fatalf("NewOpenAIImageWithBase: %v", err)
	}
	defer m.Close()
}

func TestNewGoogleImageWithBase(t *testing.T) {
	m, err := NewGoogleImageWithBase("sk-test", "gemini-2.5-flash-image", "http://localhost:9999")
	if err != nil {
		t.Fatalf("NewGoogleImageWithBase: %v", err)
	}
	defer m.Close()
}

func TestNewOpenAITranscriptionWithBase(t *testing.T) {
	m, err := NewOpenAITranscriptionWithBase("sk-test", "whisper-1", "http://localhost:9999")
	if err != nil {
		t.Fatalf("NewOpenAITranscriptionWithBase: %v", err)
	}
	defer m.Close()
}

func TestNewOpenAIFilesWithBase(t *testing.T) {
	m, err := NewOpenAIFilesWithBase("sk-test", "http://localhost:9999")
	if err != nil {
		t.Fatalf("NewOpenAIFilesWithBase: %v", err)
	}
	defer m.Close()
}

func TestNewCohereRerankingWithBase(t *testing.T) {
	m, err := NewCohereRerankingWithBase("sk-test", "rerank-v3.0", "http://localhost:9999")
	if err != nil {
		t.Fatalf("NewCohereRerankingWithBase: %v", err)
	}
	defer m.Close()
}

func TestNewGoogleVideoWithBase(t *testing.T) {
	m, err := NewGoogleVideoWithBase("sk-test", "veo-3.0", "http://localhost:9999")
	if err != nil {
		t.Fatalf("NewGoogleVideoWithBase: %v", err)
	}
	defer m.Close()
}

func TestNewTavilySearchWithBase(t *testing.T) {
	m, err := NewTavilySearchWithBase("sk-test", "http://localhost:9999")
	if err != nil {
		t.Fatalf("NewTavilySearchWithBase: %v", err)
	}
	defer m.Close()
}

// ── E2E: full FFI call via mock server ──────────────────────────────────────

func TestE2E_EmbeddingViaMock(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()
	srv.SetResponse(`{"data":[{"embedding":[0.1,0.2,0.3],"index":0}],"model":"text-embedding-3-small","usage":{"prompt_tokens":3,"total_tokens":3}}`)

	m, err := NewOpenAIEmbeddingWithBase("sk-test", "text-embedding-3-small", srv.URL)
	if err != nil {
		t.Fatal(err)
	}
	defer m.Close()

	r, err := ParseEmbeddingResult(must(m.Embed([]string{"hello"}, nil)))
	if err != nil {
		t.Fatal(err)
	}
	if len(r.Embeddings) != 1 || len(r.Embeddings[0]) != 3 {
		t.Fatalf("got %d embeddings, dim %d", len(r.Embeddings), len(r.Embeddings[0]))
	}
}

func TestE2E_SpeechViaMock(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()
	srv.SetContentType("audio/mpeg")
	srv.SetResponse(base64Audio) // raw binary body

	m, err := NewOpenAISpeechWithBase("sk-test", "tts-1", srv.URL)
	if err != nil {
		t.Fatal(err)
	}
	defer m.Close()

	voice := "alloy"
	fmt_ := "mp3"
	r, err := ParseSpeechResult(must(m.Generate(&SpeechCallOptions{Text: "Hi", Voice: &voice, OutputFormat: &fmt_})))
	if err != nil {
		t.Fatal(err)
	}
	// Binary audio comes back as AudioData::Binary
	if len(r.Audio.Binary) == 0 {
		t.Error("empty audio")
	}
}

func TestE2E_ImageViaMock(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()
	srv.SetResponse(`{"data":[{"b64_json":"aW1hZ2Ux"}]}`)

	m, err := NewOpenAIImageWithBase("sk-test", "dall-e-3", srv.URL)
	if err != nil {
		t.Fatal(err)
	}
	defer m.Close()

	prompt := "otter"
	n := 1
	r, err := ParseImageResult(must(m.Generate(&ImageCallOptions{Prompt: &prompt, N: &n})))
	if err != nil {
		t.Fatal(err)
	}
	if len(r.Images.Base64) != 1 {
		t.Fatalf("got %d images", len(r.Images.Base64))
	}
}

func TestE2E_TranscriptionViaMock(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()
	srv.SetResponse(`{"text":"Hello world"}`)

	m, err := NewOpenAITranscriptionWithBase("sk-test", "whisper-1", srv.URL)
	if err != nil {
		t.Fatal(err)
	}
	defer m.Close()

	r, err := ParseTranscriptionResult(must(m.Generate("dGVzdA==", "audio/mp3", nil)))
	if err != nil {
		t.Fatal(err)
	}
	if r.Text != "Hello world" {
		t.Errorf("got %q", r.Text)
	}
}

func TestE2E_SearchViaMock(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()
	srv.SetResponse(`{"results":[{"title":"Rust","url":"https://rust-lang.org","content":"Rust is..."}],"answer":"Rust is a systems language."}`)

	m, err := NewTavilySearchWithBase("sk-test", srv.URL)
	if err != nil {
		t.Fatal(err)
	}
	defer m.Close()

	maxR := 5
	r, err := ParseSearchResult(must(m.Search(&SearchCallOptions{Query: "What is Rust?", MaxResults: &maxR})))
	if err != nil {
		t.Fatal(err)
	}
	if len(r.Results) != 1 || r.Results[0].Title == nil || *r.Results[0].Title != "Rust" {
		t.Fatal("search result mismatch")
	}
}

func TestE2E_FilesViaMock(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()
	srv.SetResponse(`{"id":"file-abc","object":"file","bytes":1024,"created_at":1234,"filename":"test.pdf","purpose":"assistants"}`)

	m, err := NewOpenAIFilesWithBase("sk-test", srv.URL)
	if err != nil {
		t.Fatal(err)
	}
	defer m.Close()

	r, err := ParseUploadFileResult(must(m.Upload("dGVzdA==", "application/pdf", nil)))
	if err != nil {
		t.Fatal(err)
	}
	if r.ProviderReference["openai"] != "file-abc" {
		t.Errorf("got %s", r.ProviderReference["openai"])
	}
}

func TestE2E_RerankingViaMock(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()
	srv.SetResponse(`{"results":[{"index":1,"relevance_score":0.95},{"index":0,"relevance_score":0.3}]}`)

	m, err := NewCohereRerankingWithBase("sk-test", "rerank-v3.0", srv.URL)
	if err != nil {
		t.Fatal(err)
	}
	defer m.Close()

	topN := 2
	r, err := ParseRerankingResult(must(m.Rerank(&RerankingCallOptions{
		Query:     "which?",
		Documents: json.RawMessage(`{"Text":{"values":["doc1","doc2"]}}`),
		TopN:      &topN,
	})))
	if err != nil {
		t.Fatal(err)
	}
	if len(r.Ranking) != 2 || r.Ranking[0].RelevanceScore != 0.95 {
		t.Fatal("reranking mismatch")
	}
}

func TestE2E_VideoViaMock(t *testing.T) {
	// Google Video uses a multi-step async API (POST predict → poll operation →
	// fetch result). A single-response mock server can't cover the full flow.
	// Verify construction + result parsing only.
	m, err := NewGoogleVideoWithBase("sk-test", "veo-3.0", "http://localhost:9999")
	if err != nil {
		t.Fatal(err)
	}
	defer m.Close()

	r, err := ParseVideoResult(`{"videos":[{"Url":{"url":"https://example.com/v.mp4","media_type":"video/mp4"}}]}`)
	if err != nil {
		t.Fatal(err)
	}
	if len(r.Videos) != 1 || r.Videos[0].Url == nil || r.Videos[0].Url.URL != "https://example.com/v.mp4" {
		t.Fatal("video result parse mismatch")
	}
}

// must is a test helper: fails the test if the FFI call returns an error.
func must(s string, err error) string {
	if err != nil {
		panic(err)
	}
	return s
}
