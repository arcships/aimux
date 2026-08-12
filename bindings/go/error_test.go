package aimux

import (
	"encoding/json"
	"errors"
	"fmt"
	"testing"
)

// A 429 arrives as CodeAPICall; the classification is Status, and the hint
// rides along in RetryMs.
func TestErrorAs(t *testing.T) {
	inner := &Error{
		Code:    CodeAPICall,
		Message: "API call error: HTTP 429: slow down",
		Status:  429,
		RetryMs: 1000,
	}
	wrapped := fmt.Errorf("call failed: %w", inner)

	var e *Error
	if !errors.As(wrapped, &e) {
		t.Fatal("errors.As(*Error) failed")
	}
	if e.Code != CodeAPICall {
		t.Fatalf("Code: got %v", e.Code)
	}
	if e.Status != 429 || e.RetryMs != 1000 {
		t.Fatalf("Status/RetryMs: got %d / %d", e.Status, e.RetryMs)
	}
	if e.Code.String() != "ApiCall" {
		t.Fatalf("Code.String: %s", e.Code.String())
	}
}

// Auth (401) and model-not-found (404) are the same CodeAPICall kind, told
// apart by Status alone.
func TestAPICallClassification(t *testing.T) {
	for _, c := range []struct {
		status int
		msg    string
	}{
		{401, "API call error: HTTP 401: invalid api key"},
		{404, "API call error: HTTP 404: model not found"},
		{-1, "API call error: connection reset"}, // transport: no response
	} {
		e := &Error{Code: CodeAPICall, Message: c.msg, Status: c.status, RetryMs: -1}
		if got := defaultStatus(e.Code, e.Status); got != c.status {
			t.Errorf("defaultStatus(ApiCall, %d) = %d; want %d", c.status, got, c.status)
		}
		if e.Error() != c.msg {
			t.Errorf("Error() = %q; want %q", e.Error(), c.msg)
		}
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
	// Payload is a struct now, not a plain string:
	// {"NoSuchProvider":{"provider_id":"…"}}
	raw, ok := m["NoSuchProvider"]
	if !ok {
		t.Fatalf("expected key %q, got %q", "NoSuchProvider", e.ErrorValue)
	}
	var payload struct {
		ProviderID string `json:"provider_id"`
	}
	if err := json.Unmarshal(raw, &payload); err != nil {
		t.Fatalf("NoSuchProvider payload is not a struct: %v (%s)", err, raw)
	}
	if payload.ProviderID != "no-such-provider" {
		t.Fatalf("provider_id: got %q", payload.ProviderID)
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

// TestDefaultStatus: TokenExpired is a 401 by contract; every other status is
// the observed one. Nothing is invented for ApiCall — a missing status there
// means no response arrived. A concrete status is never overwritten.
func TestDefaultStatus(t *testing.T) {
	cases := []struct {
		code     Code
		in, want int
	}{
		{CodeTokenExpired, -1, 401},
		{CodeTokenExpired, 401, 401},
		// Observed ApiCall statuses pass through untouched.
		{CodeAPICall, 429, 429},
		{CodeAPICall, 401, 401},
		{CodeAPICall, 404, 404},
		{CodeAPICall, 503, 503},
		// Transport failure: no response, so no status is fabricated.
		{CodeAPICall, -1, -1},
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

// Retryable crosses the ABI as its own field. Two ApiCall failures both report
// Status -1 and disagree about retrying, so Status cannot stand in for it.
func TestRetryableIsNotDerivedFromStatus(t *testing.T) {
	transport := &Error{
		Code:      CodeAPICall,
		Message:   "API call error: connection reset",
		Status:    -1,
		RetryMs:   -1,
		Retryable: true, // request went out
	}
	missingKey := &Error{
		Code:    CodeAPICall,
		Message: "API call error: missing api key",
		Status:  -1,
		RetryMs: -1,
	} // request never went out; Retryable stays false

	if transport.Status != -1 || missingKey.Status != -1 {
		t.Fatalf("both must carry Status -1: got %d / %d", transport.Status, missingKey.Status)
	}
	if !transport.Retryable || missingKey.Retryable {
		t.Fatalf("Retryable: transport=%v missingKey=%v; want true / false",
			transport.Retryable, missingKey.Retryable)
	}
}
