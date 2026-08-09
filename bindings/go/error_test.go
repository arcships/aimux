package aimux

import (
	"encoding/json"
	"errors"
	"fmt"
	"testing"
)

func TestErrorAs(t *testing.T) {
	inner := &Error{
		Code:    CodeRateLimited,
		Message: "rate limited: slow down (retry after 1000ms)",
		Status:  429,
		RetryMs: 1000,
	}
	wrapped := fmt.Errorf("call failed: %w", inner)

	var e *Error
	if !errors.As(wrapped, &e) {
		t.Fatal("errors.As(*Error) failed")
	}
	if e.Code != CodeRateLimited {
		t.Fatalf("Code: got %v", e.Code)
	}
	if e.Status != 429 || e.RetryMs != 1000 {
		t.Fatalf("Status/RetryMs: got %d / %d", e.Status, e.RetryMs)
	}
	if e.Code.String() != "RateLimited" {
		t.Fatalf("Code.String: %s", e.Code.String())
	}
}

func TestErrorValueEngineFailure(t *testing.T) {
	_, err := Provider("no-such-provider", "sk-test-fake-key", "some-model")
	if err == nil {
		t.Fatal("expected error for unknown provider")
	}
	var e *Error
	if !errors.As(err, &e) {
		t.Fatalf("expected *Error, got %T: %v", err, err)
	}
	if e.ErrorValue == "" {
		t.Fatal("expected non-empty ErrorValue for engine failure")
	}
	var m map[string]json.RawMessage
	if err := json.Unmarshal([]byte(e.ErrorValue), &m); err != nil {
		t.Fatalf("ErrorValue is not valid JSON: %v (%q)", err, e.ErrorValue)
	}
	if len(m) != 1 {
		t.Fatalf("expected exactly one key, got %d: %q", len(m), e.ErrorValue)
	}
	if _, ok := m["UnknownProvider"]; !ok {
		t.Fatalf("expected key %q, got %q", "UnknownProvider", e.ErrorValue)
	}
}

func TestErrorValueSynthesizedFailure(t *testing.T) {
	m := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	m.Close()

	_, err := m.GenerateText(`"hello"`, "")
	if err == nil {
		t.Fatal("expected error after close")
	}
	var e *Error
	if !errors.As(err, &e) {
		t.Fatalf("expected *Error, got %T: %v", err, err)
	}
	if e.ErrorValue != "" {
		t.Fatalf("expected empty ErrorValue for FFI-synthesized failure, got %q", e.ErrorValue)
	}
}

// TestDefaultStatus verifies R4-2: when C reports status == -1, well-known
// HTTP error kinds get the conventional default status (matching Kotlin /
// Flutter / Node). A concrete status is never overwritten.
func TestDefaultStatus(t *testing.T) {
	cases := []struct {
		code     Code
		in, want int
	}{
		{CodeRateLimited, -1, 429},
		{CodeAuth, -1, 401},
		{CodeTokenExpired, -1, 401},
		{CodeModelNotFound, -1, 404},
		// Concrete statuses are preserved.
		{CodeRateLimited, 503, 503},
		{CodeAuth, 403, 403},
		{CodeModelNotFound, 404, 404},
		// Non-HTTP kinds keep -1.
		{CodeTimeout, -1, -1},
		{CodeAborted, -1, -1},
		{CodeInvalidArgument, -1, -1},
	}
	for _, c := range cases {
		got := defaultStatus(c.code, c.in)
		if got != c.want {
			t.Errorf("defaultStatus(%s, %d) = %d; want %d", c.code, c.in, got, c.want)
		}
	}
}
