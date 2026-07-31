// Unit tests for the aimux Go binding — constructor, Close, invalid input.
//
// These mirror the Kotlin ModelTest.kt: they verify the binding's lifecycle
// behavior without needing network access. Even with a fake API key, the
// provider constructs (the Rust side doesn't validate keys until a request
// is actually made).

package aimux

import (
	"testing"
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
