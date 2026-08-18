// Tests for the typed text-generation API (Generate/Stream).
// Verifies that typed inputs (string, []ModelMessage, *GenerateTextOptions)
// are correctly marshaled and typed outputs (*GenerateTextResult, typed
// StreamPart channel) are correctly produced.

package aimux

import (
	"context"
	"encoding/json"
	"errors"
	"testing"
	"time"
)

func TestTypedGenerateString(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()
	srv.SetResponse(plainOpenAIResponse)

	m := OpenAIWithBase("sk-test-fake-key", "gpt-4o", srv.URL)
	defer m.Close()

	// Typed call with plain string prompt.
	result, err := m.Generate("What is Rust?", nil)
	if err != nil {
		t.Fatalf("Generate failed: %v", err)
	}
	if result.Text != "Rust is a systems programming language." {
		t.Errorf("expected text, got %q", result.Text)
	}
}

func TestTypedGenerateMessages(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()
	srv.SetResponse(plainOpenAIResponse)

	m := OpenAIWithBase("sk-test-fake-key", "gpt-4o", srv.URL)
	defer m.Close()

	msgs := []ModelMessage{
		NewTextMessage(RoleSystem, "You are a helpful assistant."),
		NewTextMessage(RoleUser, "What is Rust?"),
	}
	result, err := m.Generate(msgs, nil)
	if err != nil {
		t.Fatalf("Generate failed: %v", err)
	}
	if result.Text != "Rust is a systems programming language." {
		t.Errorf("expected text, got %q", result.Text)
	}

	// Verify the provider received both messages.
	var reqBody map[string]any
	json.Unmarshal([]byte(srv.LastRequestBody()), &reqBody)
	messages := reqBody["messages"].([]any)
	if len(messages) != 2 {
		t.Fatalf("expected 2 messages, got %d", len(messages))
	}
}

func TestTypedGenerateWithOptions(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()
	srv.SetResponse(toolCallOpenAIResponse)

	m := OpenAIWithBase("sk-test-fake-key", "gpt-4o", srv.URL)
	defer m.Close()

	temp := 0.7
	opts := &GenerateTextOptions{
		Temperature: &temp,
		Tools: []Tool{
			{Type: "function", Name: "get_weather", InputSchema: json.RawMessage(`{"type":"object"}`)},
		},
		ToolChoice: ToolChoiceRequired(),
	}
	result, err := m.Generate("What is the weather in Tokyo?", opts)
	if err != nil {
		t.Fatalf("Generate failed: %v", err)
	}
	if len(result.ToolCalls) == 0 {
		t.Fatal("expected tool calls")
	}
	if result.ToolCalls[0].ToolName != "get_weather" {
		t.Errorf("expected 'get_weather', got %q", result.ToolCalls[0].ToolName)
	}

	// Verify the provider received the tool_choice.
	var reqBody map[string]any
	json.Unmarshal([]byte(srv.LastRequestBody()), &reqBody)
	if reqBody["tool_choice"] != "required" {
		t.Errorf("expected tool_choice 'required', got %v", reqBody["tool_choice"])
	}
}

func TestTypedGenerateRejectsInvalidPrompt(t *testing.T) {
	m := OpenAI("sk-test-fake-key", "gpt-4o")
	defer m.Close()

	_, err := m.Generate(12345, nil) // int is not a valid prompt
	if err == nil {
		t.Fatal("expected error for invalid prompt type")
	}
}

func TestTypedStreamTextDeltas(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()
	srv.SetContentType("text/event-stream")
	srv.SetResponse(buildTextDeltaSSE())

	m := OpenAIWithBase("sk-test-fake-key", "gpt-4o", srv.URL)
	defer m.Close()

	stream, err := m.Stream("Say hello", nil)
	if err != nil {
		t.Fatalf("Stream failed: %v", err)
	}

	var textBuilder string
	var gotFinish bool
	for part := range stream.Parts() {
		switch part.Tag {
		case "TextDelta":
			var td TextDeltaPayload
			json.Unmarshal(part.Payload, &td)
			textBuilder += td.Delta
		case "Finish":
			gotFinish = true
		}
	}
	if err := stream.Err(); err != nil {
		t.Fatalf("stream error: %v", err)
	}
	if textBuilder != "Hello world" {
		t.Errorf("expected 'Hello world', got %q", textBuilder)
	}
	if !gotFinish {
		t.Error("expected a Finish part")
	}
}

func TestTypedStreamMessages(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()
	srv.SetContentType("text/event-stream")
	srv.SetResponse(buildTextDeltaSSE())

	m := OpenAIWithBase("sk-test-fake-key", "gpt-4o", srv.URL)
	defer m.Close()

	msgs := []ModelMessage{
		NewTextMessage(RoleUser, "Say hello"),
	}
	stream, err := m.Stream(msgs, nil)
	if err != nil {
		t.Fatalf("Stream failed: %v", err)
	}

	count := 0
	for range stream.Parts() {
		count++
	}
	if count == 0 {
		t.Error("expected at least one stream part")
	}
	if err := stream.Err(); err != nil {
		t.Fatalf("stream error: %v", err)
	}
}

func TestTypedStreamContextAlreadyCanceled(t *testing.T) {
	m := OpenAI("sk-test-fake-key", "gpt-4o")
	if err := m.Close(); err != nil {
		t.Fatalf("Close failed: %v", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()

	stream, err := m.StreamContext(ctx, "cancel now", nil)
	if err != nil {
		t.Fatalf("StreamContext failed: %v", err)
	}
	defer stream.Cancel()

	done := make(chan error, 1)
	go func() {
		for range stream.Parts() {
		}
		done <- stream.Err()
	}()
	select {
	case err := <-done:
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("expected context.Canceled, got %v", err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("pre-canceled typed stream did not stop")
	}

	stream.Cancel()
	stream.Cancel()
}

func TestTypedStreamCancelUnblocksFullPartsChannel(t *testing.T) {
	srv, allSent, requestDone := newBackpressureSSEServer(1200)
	defer srv.Close()
	m := OpenAIWithBase("sk-test-fake-key", "gpt-4o", srv.URL)
	defer m.Close()

	stream, err := m.Stream("fill the typed channel", nil)
	if err != nil {
		t.Fatalf("Stream failed: %v", err)
	}
	defer stream.Cancel()

	select {
	case <-allSent:
	case <-time.After(2 * time.Second):
		t.Fatal("provider did not send the SSE burst")
	}
	waitForChannelFull(t, stream.parts)

	stream.Cancel()
	stream.Cancel()
	done := make(chan error, 1)
	go func() {
		for range stream.Parts() {
		}
		done <- stream.Err()
	}()
	select {
	case err := <-done:
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("expected context.Canceled, got %v", err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("full typed parts channel stayed blocked after cancellation")
	}
	select {
	case <-requestDone:
	case <-time.After(2 * time.Second):
		t.Fatal("provider request stayed open after typed cancellation")
	}
}

func TestDeepSeekFactory(t *testing.T) {
	m := DeepSeek("sk-test-fake-key", "deepseek-chat")
	defer m.Close()
	if m == nil {
		t.Fatal("expected non-nil model")
	}
	if m.id.Load() == 0 {
		t.Fatal("expected non-zero handle")
	}
}

func TestNewDeepSeekFactory(t *testing.T) {
	m, err := NewDeepSeek("sk-test-fake-key", "deepseek-chat")
	if err != nil {
		t.Fatalf("NewDeepSeek failed: %v", err)
	}
	defer m.Close()
}
