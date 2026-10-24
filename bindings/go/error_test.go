package aimux

import (
	"errors"
	"testing"
)

func TestErrorUnwrapWalksThePayloadCause(t *testing.T) {
	err, decodeErr := decodeError([]byte(`{"kind":"aimux","code":11,"message":"API call error: Cannot connect",
		"error":{"name":"AI_APICallError","message":"Cannot connect","cause":
		{"name":"AI_JSONParseError","message":"tcp connect error","cause":{"message":"Connection refused"}}}}`))
	if decodeErr != nil {
		t.Fatal(decodeErr)
	}
	var messages []string
	for link := errors.Unwrap(err); link != nil; link = errors.Unwrap(link) {
		messages = append(messages, link.Error())
	}
	if len(messages) != 2 || messages[0] != "tcp connect error" || messages[1] != "Connection refused" {
		t.Fatalf("chain %q", messages)
	}
	var cause *Cause
	if !errors.As(err.Unwrap(), &cause) || cause.Name != "AI_JSONParseError" || len(cause.Raw) == 0 {
		t.Fatalf("cause %+v", cause)
	}
	if cause.Cause.Name != "" {
		t.Fatalf("an OS error has no name: %+v", cause.Cause)
	}
	if local := localError("protocol", 301, "InvalidParams", "bad"); local.Unwrap() != nil {
		t.Fatal("no cause must unwrap to nil")
	}
}

func TestRetryErrorUnwrapsToItsLastAttempt(t *testing.T) {
	err, decodeErr := decodeError([]byte(`{"kind":"aimux","code":10,"message":"r","error":{"name":"AI_RetryError",
		"message":"r","reason":"maxRetriesExceeded","errors":[{"name":"AI_APICallError","message":"first"},
		{"name":"AI_APICallError","message":"last"}]}}`))
	if decodeErr != nil {
		t.Fatal(decodeErr)
	}
	var cause *Cause
	if !errors.As(err.Unwrap(), &cause) || cause.Name != "AI_APICallError" || cause.Message != "last" {
		t.Fatalf("cause %+v", cause)
	}
}
