package aimux

// RFC-0016 第一批 e2e tests (Go shell):
//   M2 include_raw_chunks -> StreamPart.Raw emission
//   M10 Usage.raw preservation
//   M9 StartStep non-empty warnings
//
// Mirrors the Rust chat-provider tests at
// the Go binding level, driving the real FFI path against a mock SSE server.

import (
	"encoding/json"
	"strings"
	"testing"
)

func boolPtr(b bool) *bool      { return &b }
func f64Ptr(f float64) *float64 { return &f }

// RFC-0016 M2: include_raw_chunks=true yields one Raw part per JSON SSE event,
// before the parsed parts ([DONE] excluded).
func TestE2E_StreamTextEmitsRawWhenEnabled(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()
	srv.SetContentType("text/event-stream")
	srv.SetResponse(buildChatTextDeltaSSE())

	m, err := ProviderWithBase("deepseek", "sk-test-fake-key", "deepseek-chat", srv.URL)
	if err != nil {
		t.Fatalf("ProviderWithBase: %v", err)
	}
	defer m.Close()

	stream := m.StreamText(`"Say hello"`, mustMarshalOptions(t, &GenerateTextOptions{
		IncludeRawChunks: boolPtr(true),
	}))

	var raws []json.RawMessage
	rawBeforeFirstDelta := false
	sawDelta := false
	var textBuilder strings.Builder
	for part := range stream.Parts() {
		sp, err := ParseStreamPart(part)
		if err != nil {
			t.Fatalf("failed to parse: %v", err)
		}
		switch sp.Tag {
		case "Raw":
			raws = append(raws, sp.Payload)
			if !sawDelta {
				rawBeforeFirstDelta = true
			}
		case "TextDelta":
			sawDelta = true
			if !rawBeforeFirstDelta {
				t.Error("Raw must precede the first parsed TextDelta")
			}
			var td TextDeltaPayload
			json.Unmarshal(sp.Payload, &td)
			textBuilder.WriteString(td.Text)
		}
	}
	if err := stream.Err(); err != nil {
		t.Fatalf("stream error: %v", err)
	}

	// buildChatTextDeltaSSE has 3 JSON events (2 content + 1 usage chunk);
	// [DONE] emits no Raw.
	if len(raws) != 3 {
		t.Fatalf("expected 3 Raw parts, got %d", len(raws))
	}
	var first map[string]any
	if err := json.Unmarshal(raws[0], &first); err != nil {
		t.Fatalf("Raw payload not an object: %v", err)
	}
	// Raw payload is wrapped: {"raw_value": {chunk}}.
	inner, ok := first["raw_value"].(map[string]any)
	if !ok {
		t.Fatalf("Raw payload missing raw_value, got %#v", first)
	}
	delta, ok := inner["choices"].([]any)[0].(map[string]any)["delta"].(map[string]any)["content"].(string)
	if !ok || delta != "Hello" {
		t.Errorf("first Raw should carry the 'Hello' chunk, got %#v", inner)
	}
	if textBuilder.String() != "Hello world" {
		t.Errorf("text deltas still parsed: expected 'Hello world', got %q", textBuilder.String())
	}
}

func TestE2E_StartStepCarriesNonEmptyWarnings(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()
	srv.SetContentType("text/event-stream")
	srv.SetResponse(buildChatTextDeltaSSE())

	m, err := ProviderWithBase("deepseek", "sk-test-fake-key", "deepseek-chat", srv.URL)
	if err != nil {
		t.Fatalf("ProviderWithBase: %v", err)
	}
	defer m.Close()

	// DeepSeek does not support top_k, so it must surface the warning through
	// the Go FFI stream while continuing to parse text normally.
	stream := m.StreamText(`"Say hello"`, mustMarshalOptions(t, &GenerateTextOptions{
		TopK: f64Ptr(0.5),
	}))

	sawStartStep := false
	for part := range stream.Parts() {
		sp, err := ParseStreamPart(part)
		if err != nil {
			t.Fatalf("failed to parse: %v", err)
		}
		if sp.Tag == "StartStep" {
			sawStartStep = true
			var start struct {
				Warnings []json.RawMessage `json:"warnings"`
			}
			if err := json.Unmarshal(sp.Payload, &start); err != nil {
				t.Fatalf("failed to decode StartStep payload: %v", err)
			}
			if len(start.Warnings) == 0 {
				t.Error("expected an unsupported topK warning")
			}
		}
	}
	if err := stream.Err(); err != nil {
		t.Fatalf("stream error: %v", err)
	}
	if !sawStartStep {
		t.Fatal("no StartStep part received")
	}
}

func mustMarshalOptions(t *testing.T, opts *GenerateTextOptions) string {
	t.Helper()
	s, err := MarshalOptions(opts)
	if err != nil {
		t.Fatalf("MarshalOptions: %v", err)
	}
	return s
}
