// Typed text-generation API — erases the JSON-string boundary so callers
// use typed inputs (string | []ModelMessage, *GenerateTextOptions) and get
// typed outputs (*GenerateTextResult, typed StreamPart channel).
//
// Mirrors Node's src/index.ts generateText/streamText typed wrapper.
// The raw GenerateText/StreamText methods (which accept JSON strings) remain
// available as the escape hatch.

package aimux

import (
	"context"
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

// GenerateObj is the typed object-generation method (M12, RFC-0016).
//
// Same signature as Generate; returns a typed *GenerateObjectResult. Pass
// response_format: { "Json": { ... } } via opts for schema control; the
// engine applies JSON repair before parsing.
func (m *Model) GenerateObj(prompt any, opts *GenerateTextOptions) (*GenerateObjectResult, error) {
	promptJSON, err := marshalPrompt(prompt)
	if err != nil {
		return nil, err
	}
	optsJSON, err := MarshalOptions(opts)
	if err != nil {
		return nil, err
	}
	resultJSON, err := m.GenerateObject(promptJSON, optsJSON)
	if err != nil {
		return nil, err
	}
	return ParseGenerateObjectResult(resultJSON)
}

// ConsumeStream is the typed stream-aggregation method (M11, RFC-0016).
//
// Drives stream_text to completion and returns the aggregated
// *StreamTextResultAggregated (the fully-consumed stream summary).
func (m *Model) ConsumeStream(prompt any, opts *GenerateTextOptions) (*StreamTextResultAggregated, error) {
	promptJSON, err := marshalPrompt(prompt)
	if err != nil {
		return nil, err
	}
	optsJSON, err := MarshalOptions(opts)
	if err != nil {
		return nil, err
	}
	resultJSON, err := m.ConsumeStreamText(promptJSON, optsJSON)
	if err != nil {
		return nil, err
	}
	return ParseStreamTextResultAggregated(resultJSON)
}

// TypedStream is a handle to an in-progress typed stream.
// Consume parts via Parts(); each part is a parsed *StreamPart.
type TypedStream struct {
	raw   *Stream
	parts chan *StreamPart
	err   error
	done  chan struct{}
}

// Parts returns a channel of parsed *StreamPart values.
// The channel is closed when the stream ends.
func (s *TypedStream) Parts() <-chan *StreamPart { return s.parts }

// Err returns any error that occurred during streaming.
func (s *TypedStream) Err() error {
	<-s.done
	return s.err
}

// Cancel stops this stream. It is safe to call more than once.
func (s *TypedStream) Cancel() { s.raw.Cancel() }

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
	return m.StreamContext(context.Background(), prompt, opts)
}

// StreamContext starts a typed stream that stops when ctx is cancelled.
func (m *Model) StreamContext(
	ctx context.Context,
	prompt any,
	opts *GenerateTextOptions,
) (*TypedStream, error) {
	promptJSON, err := marshalPrompt(prompt)
	if err != nil {
		return nil, err
	}
	optsJSON, err := MarshalOptions(opts)
	if err != nil {
		return nil, err
	}

	rawStream := m.StreamTextContext(ctx, promptJSON, optsJSON)
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
				rawStream.Cancel()
				return
			}
			select {
			case ts.parts <- sp:
			case <-rawStream.entry.cancelled:
				ts.err = rawStream.Err()
				return
			}
		}
		if err := rawStream.Err(); err != nil {
			ts.err = err
		}
	}()

	return ts, nil
}

// ── OpenAI Chat Completions output (RFC-0026) ──────────────────────────────

// GenerateAsOpenAI is the typed OpenAI Chat Completion generation method. It
// accepts a plain string prompt or a []ModelMessage conversation, plus optional
// typed options, and returns a typed *ChatCompletion.
//
// This is the OpenAI-output equivalent of Generate.
//
//	result, err := model.GenerateAsOpenAI("What is Rust?", nil)
//	fmt.Println(result.Choices[0].Message.Content)
func (m *Model) GenerateAsOpenAI(prompt any, opts *GenerateTextOptions) (*ChatCompletion, error) {
	promptJSON, err := marshalPrompt(prompt)
	if err != nil {
		return nil, err
	}
	optsJSON, err := MarshalOptions(opts)
	if err != nil {
		return nil, err
	}
	resultJSON, err := m.GenerateTextAsOpenAI(promptJSON, optsJSON)
	if err != nil {
		return nil, err
	}
	return ParseChatCompletion(resultJSON)
}

// OpenAIStream is a handle to an in-progress typed OpenAI stream.
// Consume parts via Parts(); each part is a parsed *ChatCompletionChunk.
type OpenAIStream struct {
	raw   *Stream
	parts chan *ChatCompletionChunk
	err   error
	done  chan struct{}
}

// Parts returns a channel of parsed *ChatCompletionChunk values.
// The channel is closed when the stream ends.
func (s *OpenAIStream) Parts() <-chan *ChatCompletionChunk { return s.parts }

// Err returns any error that occurred during streaming.
func (s *OpenAIStream) Err() error {
	<-s.done
	return s.err
}

// Cancel stops this stream. It is safe to call more than once.
func (s *OpenAIStream) Cancel() { s.raw.Cancel() }

// StreamAsOpenAI is the typed OpenAI streaming method. It accepts a plain
// string prompt or a []ModelMessage conversation, plus optional typed options,
// and returns an *OpenAIStream that yields parsed *ChatCompletionChunk values.
//
// This is the OpenAI-output equivalent of Stream.
//
//	stream, err := model.StreamAsOpenAI("Write a haiku", nil)
//	for chunk := range stream.Parts() {
//	    if len(chunk.Choices) > 0 {
//	        if c := chunk.Choices[0].Delta.Content; c != nil {
//	            fmt.Print(*c)
//	        }
//	    }
//	}
func (m *Model) StreamAsOpenAI(prompt any, opts *GenerateTextOptions) (*OpenAIStream, error) {
	return m.StreamAsOpenAIContext(context.Background(), prompt, opts)
}

// StreamAsOpenAIContext starts a typed OpenAI stream that stops when ctx is
// cancelled.
func (m *Model) StreamAsOpenAIContext(
	ctx context.Context,
	prompt any,
	opts *GenerateTextOptions,
) (*OpenAIStream, error) {
	promptJSON, err := marshalPrompt(prompt)
	if err != nil {
		return nil, err
	}
	optsJSON, err := MarshalOptions(opts)
	if err != nil {
		return nil, err
	}

	rawStream := m.StreamTextAsOpenAIContext(ctx, promptJSON, optsJSON)
	ts := &OpenAIStream{
		raw:   rawStream,
		parts: make(chan *ChatCompletionChunk, 256),
		done:  make(chan struct{}),
	}

	go func() {
		defer close(ts.done)
		defer close(ts.parts)
		for raw := range rawStream.Parts() {
			chunk, err := ParseChatCompletionChunk(raw)
			if err != nil {
				ts.err = err
				rawStream.Cancel()
				return
			}
			select {
			case ts.parts <- chunk:
			case <-rawStream.entry.cancelled:
				ts.err = rawStream.Err()
				return
			}
		}
		if err := rawStream.Err(); err != nil {
			ts.err = err
		}
	}()

	return ts, nil
}
