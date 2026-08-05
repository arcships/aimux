// Structured end-to-end tests for the Go binding.
//
// These spin up a local mock HTTP server speaking the OpenAI chat-completions
// wire format, point an OpenAI Model at it via the base_url constructor,
// and assert that the binding correctly:
//   1. parses text from a provider response,
//   2. parses tool_calls out of a provider response,
//   3. forwards multi-role messages,
//   4. forwards tool_choice,
//   5. parses tool-call stream parts from an SSE stream,
//   6. yields text-delta stream parts.
//
// No real network access is performed — every request hits 127.0.0.1.
// Mirrors the Kotlin StructuredE2ETest.kt.

package aimux

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"
)

// ── Mock provider server ──────────────────────────────────────────────────────

// mockProviderServer is a tiny HTTP server that records the last request body
// and replays a preset response. Listens on a random loopback port.
type mockProviderServer struct {
	*httptest.Server

	mu              sync.Mutex
	lastRequestBody string
	lastMethod      string

	status       int
	contentType  string
	responseBody string
	queue        []string
}

func newMockServer() *mockProviderServer {
	s := &mockProviderServer{
		status:      http.StatusOK,
		contentType: "application/json",
	}
	s.Server = httptest.NewServer(http.HandlerFunc(s.handle))
	return s
}

func (s *mockProviderServer) handle(w http.ResponseWriter, r *http.Request) {
	body, err := io.ReadAll(r.Body)
	if err != nil {
		body = nil
	}

	s.mu.Lock()
	s.lastRequestBody = string(body)
	s.lastMethod = r.Method
	status := s.status
	ct := s.contentType
	var resp string
	if len(s.queue) > 0 {
		resp = s.queue[0]
		s.queue = s.queue[1:]
	} else {
		resp = s.responseBody
	}
	s.mu.Unlock()

	w.Header().Set("Content-Type", ct)
	if strings.Contains(ct, "text/event-stream") {
		// SSE: use chunked transfer (Flusher) so the streaming reader sees
		// events as they arrive.
		w.WriteHeader(status)
		flusher, _ := w.(http.Flusher)
		fmt.Fprint(w, resp)
		if flusher != nil {
			flusher.Flush()
		}
	} else {
		w.WriteHeader(status)
		w.Write([]byte(resp))
	}
}

func (s *mockProviderServer) setResponses(responses ...string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.queue = append(s.queue[:0], responses...)
}

func (s *mockProviderServer) LastRequestBody() string {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.lastRequestBody
}

// SetResponse sets the response body and content type (thread-safe).
func (s *mockProviderServer) SetResponse(body string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.responseBody = body
}

// SetContentType sets the response content type (thread-safe).
func (s *mockProviderServer) SetContentType(ct string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.contentType = ct
}

// ── Canned OpenAI responses ────────────────────────────────────────────────────

// plainOpenAIResponse is a plain OpenAI text response (no tool calls).
const plainOpenAIResponse = `{
  "id": "chatcmpl-test",
  "model": "gpt-4o",
  "choices": [{
    "message": {"role": "assistant", "content": "Rust is a systems programming language."},
    "finish_reason": "stop"
  }],
  "usage": {"prompt_tokens": 10, "completion_tokens": 8, "total_tokens": 18}
}`

// toolCallOpenAIResponse is an OpenAI response that requests a tool call.
const toolCallOpenAIResponse = `{
  "id": "chatcmpl-tc",
  "model": "gpt-4o",
  "choices": [{
    "message": {
      "role": "assistant",
      "content": null,
      "tool_calls": [{
        "id": "call_abc",
        "type": "function",
        "function": {"name": "get_weather", "arguments": "{\"location\":\"Tokyo\"}"}
      }]
    },
    "finish_reason": "tool_calls"
  }],
  "usage": {"prompt_tokens": 20, "completion_tokens": 10, "total_tokens": 30}
}`

// weatherToolsOpts builds a JSON options string with a get_weather tool.
func weatherToolsOpts(toolChoice string) string {
	opts := map[string]any{
		"tools": []map[string]any{
			{
				"type": "function",
				"name": "get_weather",
				"input_schema": map[string]any{
					"type": "object",
					"properties": map[string]any{
						"location": map[string]any{"type": "string"},
					},
				},
			},
		},
	}
	if toolChoice != "" {
		opts["tool_choice"] = toolChoice
	}
	b, _ := json.Marshal(opts)
	return string(b)
}

// ── Tests ───────────────────────────────────────────────────────────────────

func TestE2E_GenerateTextParsesText(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()
	srv.SetResponse(plainOpenAIResponse)

	m := OpenAIWithBase("sk-test-fake-key", "gpt-4o", srv.URL)
	defer m.Close()

	result, err := m.GenerateText(`"What is Rust?"`, "")
	if err != nil {
		t.Fatalf("generate_text failed: %v", err)
	}

	// Parse the typed result.
	parsed, err := ParseGenerateTextResult(result)
	if err != nil {
		t.Fatalf("parse failed: %v", err)
	}
	if parsed.Text != "Rust is a systems programming language." {
		t.Errorf("expected text, got %q", parsed.Text)
	}
	if len(parsed.ToolCalls) != 0 {
		t.Errorf("expected no tool calls, got %d", len(parsed.ToolCalls))
	}
}

func TestE2E_GenerateTextParsesToolCalls(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()
	srv.SetResponse(toolCallOpenAIResponse)

	m := OpenAIWithBase("sk-test-fake-key", "gpt-4o", srv.URL)
	defer m.Close()

	result, err := m.GenerateText(`"What is the weather in Tokyo?"`, weatherToolsOpts(""))
	if err != nil {
		t.Fatalf("generate_text failed: %v", err)
	}

	parsed, err := ParseGenerateTextResult(result)
	if err != nil {
		t.Fatalf("parse failed: %v", err)
	}
	if len(parsed.ToolCalls) == 0 {
		t.Fatal("expected tool calls")
	}
	tc := parsed.ToolCalls[0]
	if tc.ToolName != "get_weather" {
		t.Errorf("expected tool name 'get_weather', got %q", tc.ToolName)
	}
	if tc.ToolCallID != "call_abc" {
		t.Errorf("expected tool call id 'call_abc', got %q", tc.ToolCallID)
	}

	// Verify input contains location: Tokyo.
	var input map[string]any
	json.Unmarshal(tc.Input, &input)
	if input["location"] != "Tokyo" {
		t.Errorf("expected location Tokyo, got %v", input["location"])
	}

	// raw.content should contain a "ToolCall" variant.
	var rawWrap struct {
		Raw struct {
			Content []map[string]json.RawMessage `json:"content"`
		} `json:"raw"`
	}
	json.Unmarshal([]byte(result), &rawWrap)
	hasToolCallVariant := false
	for _, part := range rawWrap.Raw.Content {
		if _, ok := part["ToolCall"]; ok {
			hasToolCallVariant = true
		}
	}
	if !hasToolCallVariant {
		t.Error("expected raw.content to contain a ToolCall variant")
	}
}

func TestE2E_MultiRoleMessagesReachProvider(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()
	srv.SetResponse(plainOpenAIResponse)

	m := OpenAIWithBase("sk-test-fake-key", "gpt-4o", srv.URL)
	defer m.Close()

	msgs := []ModelMessage{
		{Role: RoleSystem, Content: "You are a helpful assistant."},
		{Role: RoleUser, Content: "What is Rust?"},
	}
	prompt, _ := MarshalMessages(msgs)

	result, err := m.GenerateText(prompt, "")
	if err != nil {
		t.Fatalf("generate_text failed: %v", err)
	}

	parsed, _ := ParseGenerateTextResult(result)
	if parsed.Text != "Rust is a systems programming language." {
		t.Errorf("expected text, got %q", parsed.Text)
	}

	// Verify the provider received both messages.
	var reqBody map[string]any
	json.Unmarshal([]byte(srv.LastRequestBody()), &reqBody)
	if reqBody["model"] != "gpt-4o" {
		t.Errorf("expected model gpt-4o, got %v", reqBody["model"])
	}
	messages := reqBody["messages"].([]any)
	if len(messages) != 2 {
		t.Fatalf("expected 2 messages, got %d", len(messages))
	}
	first := messages[0].(map[string]any)
	if first["role"] != "system" {
		t.Errorf("expected first role 'system', got %v", first["role"])
	}
	if first["content"] != "You are a helpful assistant." {
		t.Errorf("expected first content, got %v", first["content"])
	}
	second := messages[1].(map[string]any)
	if second["role"] != "user" {
		t.Errorf("expected second role 'user', got %v", second["role"])
	}
}

func TestE2E_ToolChoiceReachesProvider(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()
	srv.SetResponse(toolCallOpenAIResponse)

	m := OpenAIWithBase("sk-test-fake-key", "gpt-4o", srv.URL)
	defer m.Close()

	_, err := m.GenerateText(`"What is the weather in Tokyo?"`, weatherToolsOpts("required"))
	if err != nil {
		t.Fatalf("generate_text failed: %v", err)
	}

	var reqBody map[string]any
	json.Unmarshal([]byte(srv.LastRequestBody()), &reqBody)
	if reqBody["tool_choice"] != "required" {
		t.Errorf("expected tool_choice 'required', got %v", reqBody["tool_choice"])
	}
	if reqBody["tools"] == nil {
		t.Error("expected tools to be forwarded")
	}
}

func TestE2E_StreamTextParsesToolCallStreamParts(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()
	srv.SetContentType("text/event-stream")

	// SSE stream: tool_calls delta (name) -> arguments delta -> finish_reason.
	sseResponse := buildToolCallSSE()
	srv.SetResponse(sseResponse)

	m := OpenAIWithBase("sk-test-fake-key", "gpt-4o", srv.URL)
	defer m.Close()

	stream := m.StreamText(`"What is the weather in Tokyo?"`, weatherToolsOpts(""))

	var parts []string
	for part := range stream.Parts() {
		parts = append(parts, part)
	}
	if err := stream.Err(); err != nil {
		t.Fatalf("stream error: %v", err)
	}
	if len(parts) == 0 {
		t.Fatal("expected at least one stream part")
	}

	// The first part should be a StreamStart or ToolCall stream part.
	// We just verify we got JSON objects with known tags.
	for _, p := range parts {
		sp, err := ParseStreamPart(p)
		if err != nil {
			t.Errorf("failed to parse stream part: %v", err)
			continue
		}
		// Tags we expect: StreamStart, TextDelta, ToolCall, Finish, etc.
		if sp.Tag == "" {
			t.Error("expected non-empty stream part tag")
		}
	}
}

func TestE2E_StreamTextYieldsTextDeltas(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()
	srv.SetContentType("text/event-stream")

	// SSE stream: "Hello" -> " world" -> finish.
	sseResponse := buildTextDeltaSSE()
	srv.SetResponse(sseResponse)

	m := OpenAIWithBase("sk-test-fake-key", "gpt-4o", srv.URL)
	defer m.Close()

	stream := m.StreamText(`"Say hello"`, "")

	var textBuilder strings.Builder
	var gotFinish bool
	for part := range stream.Parts() {
		sp, err := ParseStreamPart(part)
		if err != nil {
			t.Errorf("failed to parse: %v", err)
			continue
		}
		switch sp.Tag {
		case "TextDelta":
			var td TextDeltaPayload
			json.Unmarshal(sp.Payload, &td)
			textBuilder.WriteString(td.Delta)
		case "Finish":
			gotFinish = true
		}
	}
	if err := stream.Err(); err != nil {
		t.Fatalf("stream error: %v", err)
	}

	text := textBuilder.String()
	if text != "Hello world" {
		t.Errorf("expected 'Hello world', got %q", text)
	}
	if !gotFinish {
		t.Error("expected a Finish part")
	}
}

func TestStreamTextContextCancelsInFlightRequest(t *testing.T) {
	started := make(chan struct{})
	requestDone := make(chan struct{})
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		defer close(requestDone)
		_, _ = io.ReadAll(r.Body)
		w.Header().Set("Content-Type", "text/event-stream")
		w.WriteHeader(http.StatusOK)
		if flusher, ok := w.(http.Flusher); ok {
			flusher.Flush()
		}
		close(started)
		select {
		case <-r.Context().Done():
		case <-time.After(3 * time.Second):
		}
	}))
	defer srv.Close()

	m := OpenAIWithBase("sk-test-fake-key", "gpt-4o", srv.URL)
	defer m.Close()
	ctx, cancel := context.WithCancel(context.Background())
	stream := m.StreamTextContext(ctx, `"wait"`, "")

	select {
	case <-started:
	case <-time.After(2 * time.Second):
		t.Fatal("stream request did not start")
	}
	cancel()

	drained := make(chan struct{})
	go func() {
		for range stream.Parts() {
		}
		close(drained)
	}()
	select {
	case <-drained:
	case <-time.After(2 * time.Second):
		t.Fatal("in-flight stream did not stop after cancellation")
	}
	if !errors.Is(stream.Err(), context.Canceled) {
		t.Fatalf("expected context.Canceled, got %v", stream.Err())
	}
	select {
	case <-requestDone:
	case <-time.After(2 * time.Second):
		t.Fatal("provider request stayed open after cancellation")
	}
}

// ── SSE builders ───────────────────────────────────────────────────────────────

func buildTextDeltaSSE() string {
	chunk1 := `{"id":"1","model":"gpt-4o","choices":[{"delta":{"role":"assistant","content":"Hello"}}]}`
	chunk2 := `{"id":"1","model":"gpt-4o","choices":[{"delta":{"content":" world"}}]}`
	chunk3 := `{"id":"1","model":"gpt-4o","choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}`
	var sb strings.Builder
	sb.WriteString("data: " + chunk1 + "\n\n")
	sb.WriteString("data: " + chunk2 + "\n\n")
	sb.WriteString("data: " + chunk3 + "\n\n")
	sb.WriteString("data: [DONE]\n\n")
	return sb.String()
}

func buildToolCallSSE() string {
	chunk1 := `{"id":"1","model":"gpt-4o","choices":[{"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_xyz","type":"function","function":{"name":"get_weather","arguments":""}}]}}]}`
	chunk2 := `{"id":"1","model":"gpt-4o","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"location\":\"Tokyo\"}"}}]}}]}`
	chunk3 := `{"id":"1","model":"gpt-4o","choices":[{"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":5,"completion_tokens":2,"total_tokens":7}}`
	var sb strings.Builder
	sb.WriteString("data: " + chunk1 + "\n\n")
	sb.WriteString("data: " + chunk2 + "\n\n")
	sb.WriteString("data: " + chunk3 + "\n\n")
	sb.WriteString("data: [DONE]\n\n")
	return sb.String()
}

// ── Tool-call full round-trip ─────────────────────────────────────────────────

// TestE2E_ToolCallFullRoundTrip mirrors Kotlin's `tool-call full round-trip`
// scenario: the model requests a tool call, the client sends the tool result
// back as a multi-part tool message, and the model produces a final answer.
// This verifies ModelMessage multi-part content reaches the provider correctly.
func TestE2E_ToolCallFullRoundTrip(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()

	// Queue: first call returns a tool_call, second call returns the final text.
	srv.setResponses(toolCallOpenAIResponse, plainOpenAIResponse)

	m := OpenAIWithBase("sk-test-fake-key", "gpt-4o", srv.URL)
	defer m.Close()

	// 1. First call: model requests get_weather tool call.
	result1, err := m.GenerateText(`"What is the weather in Tokyo?"`, weatherToolsOpts(""))
	if err != nil {
		t.Fatalf("first generate failed: %v", err)
	}
	parsed1, _ := ParseGenerateTextResult(result1)
	if len(parsed1.ToolCalls) == 0 {
		t.Fatal("expected tool calls in first response")
	}
	call := parsed1.ToolCalls[0]
	if call.ToolName != "get_weather" {
		t.Fatalf("expected tool name 'get_weather', got %q", call.ToolName)
	}

	// 2. Build a multi-part tool-result message using ModelMessage's Content
	//    (any) field — this is the path that was previously unsupported.
	toolResultMsg := ModelMessage{
		Role: RoleTool,
		Content: []map[string]any{
			{
				"type":         "tool_result",
				"tool_call_id": call.ToolCallID,
				"tool_name":    call.ToolName,
				"result":       map[string]any{"temperature": "20C"},
			},
		},
	}
	conversation := []ModelMessage{
		NewTextMessage(RoleUser, "What is the weather in Tokyo?"),
		NewTextMessage(RoleAssistant, ""), // assistant's prior turn
		toolResultMsg,                       // tool result fed back
	}
	prompt, _ := MarshalMessages(conversation)

	// 3. Second call: model produces final text from the tool result.
	result2, err := m.GenerateText(prompt, "")
	if err != nil {
		t.Fatalf("second generate failed: %v", err)
	}
	parsed2, _ := ParseGenerateTextResult(result2)
	if parsed2.Text == "" {
		t.Error("expected non-empty final text")
	}

	// Verify the second request carried the tool-result message correctly.
	// The engine converts multi-part tool_result content into OpenAI's wire
	// format: {"role":"tool","tool_call_id":"...","content":"<stringified result>"}.
	reqBody := srv.LastRequestBody()
	var body struct {
		Messages []struct {
			Role         string          `json:"role"`
			Content      json.RawMessage `json:"content"`
			ToolCallID   string          `json:"tool_call_id,omitempty"`
		} `json:"messages"`
	}
	json.Unmarshal([]byte(reqBody), &body)
	if len(body.Messages) != 3 {
		t.Fatalf("expected 3 messages, got %d", len(body.Messages))
	}
	// The third message (tool result) in OpenAI format.
	third := body.Messages[2]
	if third.Role != "tool" {
		t.Errorf("expected third role 'tool', got %q", third.Role)
	}
	if third.ToolCallID != call.ToolCallID {
		t.Errorf("expected tool_call_id %q, got %q", call.ToolCallID, third.ToolCallID)
	}
	// The content should contain the result data (stringified by the engine).
	contentStr := string(third.Content)
	if !strings.Contains(contentStr, "temperature") {
		t.Errorf("expected content to contain result data, got %s", contentStr)
	}
}
