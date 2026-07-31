// Example: minimal non-streaming text generation with aimux.
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

	"github.com/aimux/aimux-go"
)

func main() {
	apiKey := os.Getenv("OPENAI_API_KEY")
	if apiKey == "" {
		log.Fatal("OPENAI_API_KEY not set")
	}

	model := aimux.OpenAI(apiKey, "gpt-4o")
	defer model.Close()

	result, err := model.GenerateText(`"Explain Rust ownership in one sentence."`, "")
	if err != nil {
		log.Fatalf("generate failed: %v", err)
	}

	// Parse the typed result.
	parsed, err := aimux.ParseGenerateTextResult(result)
	if err != nil {
		log.Fatalf("parse failed: %v", err)
	}

	fmt.Println(parsed.Text)

	// Or use streaming:
	fmt.Println("\n--- Streaming ---")
	stream := model.StreamText(`"Write a haiku about Rust."`, "")
	for part := range stream.Parts() {
		sp, err := aimux.ParseStreamPart(part)
		if err != nil {
			continue
		}
		if sp.Tag == "TextDelta" {
			var td aimux.TextDeltaPayload
			json.Unmarshal(sp.Payload, &td)
			fmt.Print(td.Delta)
		}
	}
	if err := stream.Err(); err != nil {
		log.Fatalf("stream error: %v", err)
	}
	fmt.Println()
}
