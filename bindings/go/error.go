package aimux

import (
	"encoding/json"
	"errors"
	"fmt"
)

var (
	ErrClosed = errors.New("aimux: use of closed handle")
)

// ErrorEnvelope is the canonical envelope. Error preserves the complete
// AI SDK payload, including nested errors, provider data and isRetryable.
type ErrorEnvelope struct {
	Kind         string          `json:"kind"`
	Code         int32           `json:"code"`
	Message      string          `json:"message"`
	Error        json.RawMessage `json:"error"`
	RetryAfterMs *uint64         `json:"retryAfterMs,omitempty"`
}

// Error supports errors.As across all four domains. RawEnvelope retains all
// fields, including fields unknown to this version of the Go binding.
type Error struct {
	Envelope    ErrorEnvelope
	RawEnvelope json.RawMessage
}

func (e *Error) Error() string {
	return fmt.Sprintf("aimux %s (%d): %s", e.Envelope.Kind, e.Envelope.Code, e.Envelope.Message)
}

// Unwrap returns the payload's cause, so errors.Unwrap, errors.Is and
// errors.As walk the same chain as Rust's source().
func (e *Error) Unwrap() error {
	var payload struct {
		Cause *Cause `json:"cause"`
	}
	if json.Unmarshal(e.Envelope.Error, &payload) != nil || payload.Cause == nil {
		return nil
	}
	return payload.Cause
}

// Cause is one link of an error's cause chain: a nested AI SDK error (Name
// set, Raw holding its whole payload) or an underlying OS, TLS or parser
// error (no Name).
type Cause struct {
	Name    string
	Message string
	Cause   *Cause
	Raw     json.RawMessage
}

func (c *Cause) Error() string { return c.Message }

func (c *Cause) Unwrap() error {
	if c.Cause == nil {
		return nil
	}
	return c.Cause
}

func (c *Cause) UnmarshalJSON(data []byte) error {
	var fields struct {
		Name    string `json:"name"`
		Message string `json:"message"`
		Cause   *Cause `json:"cause"`
	}
	if err := json.Unmarshal(data, &fields); err != nil {
		return err
	}
	*c = Cause{fields.Name, fields.Message, fields.Cause, append(json.RawMessage(nil), data...)}
	return nil
}

func decodeError(data []byte) (*Error, error) {
	if err := validateJSON("error envelope", data); err != nil {
		return nil, err
	}
	var envelope ErrorEnvelope
	if err := json.Unmarshal(data, &envelope); err != nil {
		return nil, err
	}
	switch envelope.Kind {
	case "aimux", "recording", "binding", "protocol":
	default:
		return nil, fmt.Errorf("aimux: invalid error domain %q", envelope.Kind)
	}
	if envelope.Code == 0 || len(envelope.Error) == 0 || string(envelope.Error) == "null" {
		return nil, errors.New("aimux: incomplete error envelope")
	}
	return &Error{Envelope: envelope, RawEnvelope: append(json.RawMessage(nil), data...)}, nil
}

func localError(kind string, code int32, name, message string) *Error {
	payload, _ := json.Marshal(struct {
		Name    string `json:"name"`
		Message string `json:"message"`
	}{name, message})
	envelope := ErrorEnvelope{Kind: kind, Code: code, Message: message, Error: payload}
	raw, _ := json.Marshal(envelope)
	return &Error{Envelope: envelope, RawEnvelope: raw}
}
