// Structured end-to-end tests for the Go binding.
//
// These spin up a local mock HTTP server speaking the OpenAI Responses API
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
	lastPath        string

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
	s.lastPath = r.URL.Path
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

func (s *mockProviderServer) LastRequestPath() string {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.lastPath
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

// newBackpressureSSEServer sends enough parts to fill the Go channel. It then
// keeps the HTTP request open until the client cancels it.
func newBackpressureSSEServer(chunks int) (*httptest.Server, <-chan struct{}, <-chan struct{}) {
	allSent := make(chan struct{})
	requestDone := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		defer close(requestDone)
		_, _ = io.ReadAll(r.Body)
		w.Header().Set("Content-Type", "text/event-stream")
		w.WriteHeader(http.StatusOK)
		flusher, _ := w.(http.Flusher)
		_, _ = fmt.Fprint(w, "data: "+responsesTextStart+"\n\n")
		for i := 0; i < chunks; i++ {
			chunk := `{"type":"response.output_text.delta","item_id":"msg-test","output_index":0,"content_index":0,"delta":"x"}`
			_, _ = fmt.Fprintf(w, "data: %s\n\n", chunk)
			if flusher != nil {
				flusher.Flush()
			}
		}
		close(allSent)
		select {
		case <-r.Context().Done():
		case <-time.After(5 * time.Second):
		}
	}))
	return server, allSent, requestDone
}

func waitForChannelFull[T any](t *testing.T, channel chan T) {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for len(channel) < cap(channel) {
		if time.Now().After(deadline) {
			t.Fatalf("channel did not fill: got %d of %d", len(channel), cap(channel))
		}
		time.Sleep(5 * time.Millisecond)
	}
}

// ── Canned OpenAI responses ────────────────────────────────────────────────────

// plainOpenAIResponse is a plain OpenAI text response (no tool calls).
const plainOpenAIResponse = `{
  "id": "resp-test",
  "model": "gpt-4o",
  "created_at": 1,
  "status": "completed",
  "output": [{
    "type": "message", "id": "msg-test", "role": "assistant", "status": "completed",
    "content": [{"type":"output_text","text":"Rust is a systems programming language.","annotations":[]}]
  }],
  "usage": {"input_tokens": 10, "output_tokens": 8, "total_tokens": 18}
}`

// toolCallOpenAIResponse is an OpenAI response that requests a tool call.
const toolCallOpenAIResponse = `{
  "id": "resp-tc",
  "model": "gpt-4o",
  "created_at": 1,
  "status": "completed",
  "output": [{
    "type": "function_call", "id": "fc-abc", "call_id": "call_abc",
    "name": "get_weather", "arguments": "{\"location\":\"Tokyo\"}", "status": "completed"
  }],
  "usage": {"input_tokens": 20, "output_tokens": 10, "total_tokens": 30}
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
	if srv.LastRequestPath() != "/responses" {
		t.Errorf("expected /responses endpoint, got %q", srv.LastRequestPath())
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

	// content should contain a "ToolCall" variant.
	var contentWrap struct {
		Content []map[string]json.RawMessage `json:"content"`
	}
	json.Unmarshal([]byte(result), &contentWrap)
	hasToolCallVariant := false
	for _, part := range contentWrap.Content {
		if _, ok := part["ToolCall"]; ok {
			hasToolCallVariant = true
		}
	}
	if !hasToolCallVariant {
		t.Error("expected content to contain a ToolCall variant")
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
	messages := reqBody["input"].([]any)
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

	// Responses SSE: output item -> arguments delta -> completed tool -> finish.
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

	// The stream opens with Start and StartStep parts.
	var gotToolCall, gotFinish bool
	for _, p := range parts {
		sp, err := ParseStreamPart(p)
		if err != nil {
			t.Errorf("failed to parse stream part: %v", err)
			continue
		}
		if sp.Tag == "" {
			t.Error("expected non-empty stream part tag")
		}
		switch sp.Tag {
		case "ToolCall":
			gotToolCall = true
			var call ToolCall
			if err := json.Unmarshal(sp.Payload, &call); err != nil {
				t.Fatalf("decode tool call: %v", err)
			}
			if call.ToolName != "get_weather" || call.ToolCallID != "call_xyz" {
				t.Errorf("unexpected tool call: %#v", call)
			}
			var input map[string]any
			if err := json.Unmarshal(call.Input, &input); err != nil || input["location"] != "Tokyo" {
				t.Errorf("unexpected tool arguments: %q (%v)", call.Input, err)
			}
		case "Finish":
			gotFinish = true
		}
	}
	if !gotToolCall || !gotFinish {
		t.Errorf("expected completed tool call and Finish; gotToolCall=%t gotFinish=%t", gotToolCall, gotFinish)
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
			textBuilder.WriteString(td.Text)
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

func TestStreamCancelUnblocksFullPartsChannel(t *testing.T) {
	srv, allSent, requestDone := newBackpressureSSEServer(800)
	defer srv.Close()

	m := OpenAIWithBase("sk-test-fake-key", "gpt-4o", srv.URL)
	defer m.Close()
	stream := m.StreamText(`"fill the channel"`, "")
	defer stream.Cancel()

	select {
	case <-allSent:
	case <-time.After(2 * time.Second):
		t.Fatal("provider did not send the SSE burst")
	}
	waitForChannelFull(t, stream.parts)

	tasks := []func(){func() { m.Close() }}
	for i := 0; i < 32; i++ {
		tasks = append(tasks, stream.Cancel)
	}
	runConcurrent(t, "full stream buffer Cancel + Model.Close", tasks...)
	drained := make(chan struct{})
	go func() {
		for range stream.Parts() {
		}
		close(drained)
	}()
	select {
	case <-drained:
	case <-time.After(2 * time.Second):
		t.Fatal("full parts channel stayed blocked after cancellation")
	}
	if !errors.Is(stream.Err(), context.Canceled) {
		t.Fatalf("expected context.Canceled, got %v", stream.Err())
	}
	select {
	case <-requestDone:
	case <-time.After(2 * time.Second):
		t.Fatal("provider request stayed open after backpressure cancellation")
	}
}

// ── SSE builders ───────────────────────────────────────────────────────────────

const responsesTextStart = `{"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"msg-test","role":"assistant","content":[],"status":"in_progress"}}`

func responsesSSE(events ...string) string {
	var sb strings.Builder
	for _, event := range events {
		sb.WriteString("data: " + event + "\n\n")
	}
	return sb.String()
}

func buildTextDeltaSSE() string {
	return responsesSSE(
		`{"type":"response.created","response":{"id":"resp-test","model":"gpt-4o","created_at":1,"status":"in_progress","output":[]}}`,
		responsesTextStart,
		`{"type":"response.output_text.delta","item_id":"msg-test","output_index":0,"content_index":0,"delta":"Hello"}`,
		`{"type":"response.output_text.delta","item_id":"msg-test","output_index":0,"content_index":0,"delta":" world"}`,
		`{"type":"response.output_item.done","output_index":0,"item":{"type":"message","id":"msg-test","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Hello world","annotations":[]}]}}`,
		`{"type":"response.completed","response":{"id":"resp-test","model":"gpt-4o","created_at":1,"status":"completed","output":[],"usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}}}`,
	)
}

// Chat providers emit raw chunks; the Responses provider does not support them.
func buildChatTextDeltaSSE() string {
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
	return responsesSSE(
		`{"type":"response.created","response":{"id":"resp-tc","model":"gpt-4o","created_at":1,"status":"in_progress","output":[]}}`,
		`{"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"fc-xyz","call_id":"call_xyz","name":"get_weather","arguments":"","status":"in_progress"}}`,
		`{"type":"response.function_call_arguments.delta","output_index":0,"item_id":"fc-xyz","delta":"{\"location\":\"Tokyo\"}"}`,
		`{"type":"response.output_item.done","output_index":0,"item":{"type":"function_call","id":"fc-xyz","call_id":"call_xyz","name":"get_weather","arguments":"{\"location\":\"Tokyo\"}","status":"completed"}}`,
		`{"type":"response.completed","response":{"id":"resp-tc","model":"gpt-4o","created_at":1,"status":"completed","output":[],"usage":{"input_tokens":5,"output_tokens":2,"total_tokens":7}}}`,
	)
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
		{Role: RoleAssistant, Content: []map[string]any{
			{"type": "tool_call", "tool_call_id": call.ToolCallID, "tool_name": call.ToolName, "input": map[string]any{"location": "Tokyo"}},
		}},
		toolResultMsg,
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
	// Responses preserves the assistant's function_call and sends the tool's
	// JSON result as a function_call_output with the same call_id.
	reqBody := srv.LastRequestBody()
	var body struct {
		Input []struct {
			Role       string `json:"role"`
			Type       string `json:"type"`
			Name       string `json:"name"`
			Arguments  string `json:"arguments"`
			Output     string `json:"output"`
			ToolCallID string `json:"call_id"`
		} `json:"input"`
	}
	if err := json.Unmarshal([]byte(reqBody), &body); err != nil {
		t.Fatalf("decode request: %v", err)
	}
	if len(body.Input) != 3 {
		t.Fatalf("expected 3 input items, got %d", len(body.Input))
	}
	second := body.Input[1]
	if second.Type != "function_call" || second.ToolCallID != call.ToolCallID || second.Name != call.ToolName {
		t.Errorf("assistant tool call did not round-trip: %#v", second)
	}
	var arguments map[string]any
	if err := json.Unmarshal([]byte(second.Arguments), &arguments); err != nil || arguments["location"] != "Tokyo" {
		t.Errorf("assistant arguments did not round-trip: %q (%v)", second.Arguments, err)
	}
	third := body.Input[2]
	if third.Type != "function_call_output" {
		t.Errorf("expected function_call_output, got %q", third.Type)
	}
	if third.ToolCallID != call.ToolCallID {
		t.Errorf("expected tool_call_id %q, got %q", call.ToolCallID, third.ToolCallID)
	}
	var output map[string]any
	if err := json.Unmarshal([]byte(third.Output), &output); err != nil || output["temperature"] != "20C" {
		t.Errorf("tool output did not round-trip: %q (%v)", third.Output, err)
	}
}
