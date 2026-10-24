package aimux

import (
	"encoding/json"
	"errors"
	"fmt"
)

var (
	ErrClosed = errors.New("aimux: use of closed handle")
)

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
	var payload Cause
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

// UnmarshalJSON reads one link. A retry error's cause is its last attempt,
// as Rust's source() and the AI SDK lastError.
func (c *Cause) UnmarshalJSON(data []byte) error {
	var fields struct {
		Name    string   `json:"name"`
		Message string   `json:"message"`
		Cause   *Cause   `json:"cause"`
		Errors  []*Cause `json:"errors"`
	}
	if err := json.Unmarshal(data, &fields); err != nil {
		return err
	}
	if fields.Cause == nil && fields.Name == "AI_RetryError" && len(fields.Errors) > 0 {
		fields.Cause = fields.Errors[len(fields.Errors)-1]
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
	return &Error{Envelope: envelope, RawEnvelope: append(json.RawMessage(nil), data...)}, nil
}

func localError(kind string, code int32, name, message string) *Error {
	payload, _ := json.Marshal(struct {
		Name    string `json:"name"`
		Message string `json:"message"`
	}{name, message})
	envelope := ErrorEnvelope{Kind: ErrorDomain(kind), Code: code, Message: message, Error: payload}
	raw, _ := json.Marshal(envelope)
	return &Error{Envelope: envelope, RawEnvelope: raw}
}
