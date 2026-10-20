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
