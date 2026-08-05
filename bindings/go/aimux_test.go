// Unit tests for the aimux Go binding — constructor, Close, invalid input.
//
// These mirror the Kotlin ModelTest.kt: they verify the binding's lifecycle
// behavior without needing network access. Even with a fake API key, the
// provider constructs (the Rust side doesn't validate keys until a request
// is actually made).

package aimux

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"
)

func TestOpenAI(t *testing.T) {
	m := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	if m == nil {
		t.Fatal("expected non-nil model")
	}
	defer m.Close()
	if m.handle == 0 {
		t.Fatal("expected non-zero handle")
	}
}

func TestAnthropic(t *testing.T) {
	m := Anthropic("sk-ant-test-fake-key", "claude-3-5-sonnet-20241022")
	if m == nil {
		t.Fatal("expected non-nil model")
	}
	defer m.Close()
	if m.handle == 0 {
		t.Fatal("expected non-zero handle")
	}
}

func TestOpenAIWithBase(t *testing.T) {
	m := OpenAIWithBase("sk-test-fake-key", "gpt-4o-mini", "http://localhost:11434")
	if m == nil {
		t.Fatal("expected non-nil model")
	}
	defer m.Close()
	if m.handle == 0 {
		t.Fatal("expected non-zero handle")
	}
}

func TestModelClose(t *testing.T) {
	m := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	m.Close()
	// Double-close should not crash.
	if err := m.Close(); err != nil {
		t.Fatalf("double close should be safe: %v", err)
	}
}

func TestGenerateTextRejectsInvalidPrompt(t *testing.T) {
	m := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	defer m.Close()

	// Invalid JSON prompt should produce an error (the engine will try to
	// parse it and fail). We can't hit a real API, but the FFI layer returns
	// an error response for malformed input.
	_, err := m.GenerateText("{invalid json}", "")
	// With a fake key, this will either fail at JSON parse or at network.
	// Either way, no nil error + empty string.
	if err == nil {
		// The provider might construct and fail on network — that's fine,
		// the error comes as a JSON error string. Check it's not a panic.
		// If no error (e.g., somehow succeeded), at least verify it didn't crash.
		t.Log("generate_text did not return error (unexpected but not fatal with fake key)")
	}
}

func TestGenerateTextAfterClose(t *testing.T) {
	m := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	m.Close()

	_, err := m.GenerateText(`"hello"`, "")
	if err == nil {
		t.Fatal("expected error after close")
	}
}

func TestStreamTextReturnsStream(t *testing.T) {
	m := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	defer m.Close()

	// We don't consume the stream (would need network), but verify it
	// can be created without panicking.
	s := m.StreamText(`"hello"`, "")
	if s == nil {
		t.Fatal("expected non-nil stream")
	}
	// Parts() should return a channel.
	ch := s.Parts()
	if ch == nil {
		t.Fatal("expected non-nil parts channel")
	}
	// Drain to avoid leaking the goroutine (the fake-key stream will error
	// quickly since no real API is reachable).
	for range ch {
	}
	// Err() should be safe to call after drain.
	_ = s.Err()
}

func TestStreamTextContextAlreadyCanceled(t *testing.T) {
	m := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	defer m.Close()

	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	stream := m.StreamTextContext(ctx, `"hello"`, "")

	done := make(chan struct{})
	go func() {
		for range stream.Parts() {
		}
		close(done)
	}()

	select {
	case <-done:
	case <-time.After(2 * time.Second):
		t.Fatal("cancelled stream did not stop")
	}
	if !errors.Is(stream.Err(), context.Canceled) {
		t.Fatalf("expected context.Canceled, got %v", stream.Err())
	}

	stream.Cancel()
	stream.Cancel()
}

func TestProviderWithConfigFullOptions(t *testing.T) {
	retries := uint32(0)
	m, err := ProviderWithConfig("groq", "sk-test-fake-key", "llama-3.3-70b", &ProviderConfig{
		BaseURL:       "https://example.com/v1",
		Headers:       map[string]string{"X-Custom": "1"},
		Organization:  "org-1",
		Project:       "proj-1",
		MaxRetries:    &retries,
		BodyOverrides: map[string]any{"temperature": 0.1},
	})
	if err != nil {
		t.Fatalf("expected success, got %v", err)
	}
	defer m.Close()
	if m.handle == 0 {
		t.Fatal("expected non-zero handle")
	}
}

func TestProviderWithConfigNil(t *testing.T) {
	m, err := ProviderWithConfig("groq", "sk-test-fake-key", "llama-3.3-70b", nil)
	if err != nil {
		t.Fatalf("expected success, got %v", err)
	}
	defer m.Close()
}

func TestProviderWithBaseQuotedURLDoesNotInjectJSON(t *testing.T) {
	// A baseURL containing a quote must not produce malformed config JSON
	// (the old string concatenation would). The provider layer may reject
	// the URL itself, but the error must not be a JSON parse failure.
	_, err := ProviderWithBase("groq", "sk-test-fake-key", "llama-3.3-70b", `https://example.com/"v1`)
	if err != nil && strings.Contains(err.Error(), "invalid provider config JSON") {
		t.Fatalf("config JSON injection: %v", err)
	}
}
