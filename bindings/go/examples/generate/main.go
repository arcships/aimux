// Example: typed text generation and multimodal with aimux.
//
// Prerequisites:
//   cargo build -p aimux-ffi --release
//
// Run:
//   cd bindings/go
//   OPENAI_API_KEY=sk-... go run ./examples/generate

package main

import (
	"encoding/json"
	"fmt"
	"log"
	"os"

	"github.com/arcships/aimux/bindings/go"
)

func main() {
	apiKey := os.Getenv("OPENAI_API_KEY")
	if apiKey == "" {
		log.Fatal("OPENAI_API_KEY not set")
	}

	// ── Typed text generation (no manual JSON) ────────────────────────────
	model := aimux.OpenAI(apiKey, "gpt-4o")
	defer model.Close()

	result, err := model.Generate("Explain Rust ownership in one sentence.", nil)
	if err != nil {
		log.Fatalf("generate failed: %v", err)
	}
	fmt.Println(result.Text)

	// ── Typed streaming ────────────────────────────────────────────────────
	fmt.Println("\n--- Streaming ---")
	stream, err := model.Stream("Write a haiku about Rust.", nil)
	if err != nil {
		log.Fatalf("stream failed: %v", err)
	}
	for part := range stream.Parts() {
		if part.Tag == "TextDelta" {
			var td aimux.TextDeltaPayload
			json.Unmarshal(part.Payload, &td)
			fmt.Print(td.Delta)
		}
	}
	if err := stream.Err(); err != nil {
		log.Fatalf("stream error: %v", err)
	}
	fmt.Println()

	// ── Multimodal: Embedding ──────────────────────────────────────────────
	emb, err := aimux.NewOpenAIEmbedding(apiKey, "text-embedding-3-small")
	if err != nil {
		log.Fatalf("embedding model failed: %v", err)
	}
	defer emb.Close()
	// emb.Embed([]string{"hello", "world"}, nil) — requires network

	// ── DeepSeek (OpenAI-compatible convenience) ──────────────────────────
	deepseek := aimux.DeepSeek(os.Getenv("DEEPSEEK_API_KEY"), "deepseek-chat")
	defer deepseek.Close()
	// result, _ = deepseek.Generate("Hello", nil) — requires network
}
