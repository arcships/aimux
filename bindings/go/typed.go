// Typed text-generation API — erases the JSON-string boundary so callers
// use typed inputs (string | []ModelMessage, *GenerateTextOptions) and get
// typed outputs (*GenerateTextResult, typed StreamPart channel).
//
// Mirrors Node's src/index.ts generateText/streamText typed wrapper.
// The raw GenerateText/StreamText methods (which accept JSON strings) remain
// available as the escape hatch.

package aimux

import (
	"encoding/json"
	"fmt"
)

// marshalPrompt converts a typed prompt (string or []ModelMessage) to the
// JSON wire format expected by the FFI layer.
func marshalPrompt(prompt any) (string, error) {
	switch p := prompt.(type) {
	case string:
		b, err := json.Marshal(p)
		if err != nil {
			return "", fmt.Errorf("aimux: failed to marshal prompt string: %w", err)
		}
		return string(b), nil
	case []ModelMessage:
		b, err := json.Marshal(p)
		if err != nil {
			return "", fmt.Errorf("aimux: failed to marshal messages: %w", err)
		}
		return string(b), nil
	default:
		return "", fmt.Errorf("aimux: prompt must be string or []ModelMessage, got %T", prompt)
	}
}

// Generate is the typed text generation method. It accepts a plain string
// prompt or a []ModelMessage conversation, plus optional typed options,
// and returns a typed *GenerateTextResult.
//
// This is the Go equivalent of Node's generateText(model, prompt, options).
//
//	result, err := model.Generate("What is Rust?", nil)
//	fmt.Println(result.Text)
//
//	result, err := model.Generate([]ModelMessage{
//	    NewTextMessage(RoleSystem, "You are helpful."),
//	    NewTextMessage(RoleUser, "What is Rust?"),
//	}, nil)
func (m *Model) Generate(prompt any, opts *GenerateTextOptions) (*GenerateTextResult, error) {
	promptJSON, err := marshalPrompt(prompt)
	if err != nil {
		return nil, err
	}
	optsJSON, err := MarshalOptions(opts)
	if err != nil {
		return nil, err
	}
	resultJSON, err := m.GenerateText(promptJSON, optsJSON)
	if err != nil {
		return nil, err
	}
	return ParseGenerateTextResult(resultJSON)
}

// TypedStream is a handle to an in-progress typed stream.
// Consume parts via Parts(); each part is a parsed *StreamPart.
type TypedStream struct {
	raw    *Stream
	parts  chan *StreamPart
	err    error
	done   chan struct{}
}

// Parts returns a channel of parsed *StreamPart values.
// The channel is closed when the stream ends.
func (s *TypedStream) Parts() <-chan *StreamPart { return s.parts }

// Err returns any error that occurred during streaming.
func (s *TypedStream) Err() error {
	<-s.done
	return s.err
}

// Stream is the typed streaming method. It accepts a plain string prompt or
// a []ModelMessage conversation, plus optional typed options, and returns a
// *TypedStream that yields parsed *StreamPart values.
//
// This is the Go equivalent of Node's streamText(model, prompt, options).
//
//	stream, err := model.Stream("Write a haiku", nil)
//	for part := range stream.Parts() {
//	    if part.Tag == "TextDelta" {
//	        var td TextDeltaPayload
//	        json.Unmarshal(part.Payload, &td)
//	        fmt.Print(td.Delta)
//	    }
//	}
func (m *Model) Stream(prompt any, opts *GenerateTextOptions) (*TypedStream, error) {
	promptJSON, err := marshalPrompt(prompt)
	if err != nil {
		return nil, err
	}
	optsJSON, err := MarshalOptions(opts)
	if err != nil {
		return nil, err
	}

	rawStream := m.StreamText(promptJSON, optsJSON)
	ts := &TypedStream{
		raw:   rawStream,
		parts: make(chan *StreamPart, 256),
		done:  make(chan struct{}),
	}

	go func() {
		defer close(ts.done)
		defer close(ts.parts)
		for raw := range rawStream.Parts() {
			sp, err := ParseStreamPart(raw)
			if err != nil {
				ts.err = err
				return
			}
			ts.parts <- sp
		}
		if err := rawStream.Err(); err != nil {
			ts.err = err
		}
	}()

	return ts, nil
}
