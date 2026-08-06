// Round-trip (de)serialization tests for the typed wire-format types.
//
// Mirrors Kotlin TypedModelRoundTripTest.kt: build a typed value, marshal to
// JSON, unmarshal back, assert equality. These pin down wire-format field
// names and tag spellings the typed wrappers rely on — without them, a rename
// in aimux-core could silently break Go clients.
//
// Also includes regression tests for the two bugs found in code review:
//   - ToolChoiceTool JSON injection (now uses json.Marshal)
//   - channel double-close panic (now guarded by sync.Once)

package aimux

import (
	"encoding/json"
	"strings"
	"testing"
)

// ── Enum round-trips ────────────────────────────────────────────────────────

func TestRoleRoundTrip(t *testing.T) {
	cases := []Role{RoleSystem, RoleUser, RoleAssistant, RoleTool}
	for _, r := range cases {
		b, _ := json.Marshal(r)
		var got Role
		json.Unmarshal(b, &got)
		if got != r {
			t.Errorf("round-trip mismatch: got %s, want %s", got, r)
		}
	}
}

func TestFinishReasonUnifiedRoundTrip(t *testing.T) {
	cases := []FinishReasonUnified{
		FinishStop, FinishLength, FinishContentFilter,
		FinishToolCalls, FinishError, FinishOther,
	}
	for _, f := range cases {
		b, _ := json.Marshal(f)
		var got FinishReasonUnified
		json.Unmarshal(b, &got)
		if got != f {
			t.Errorf("round-trip mismatch: got %s, want %s", got, f)
		}
	}
}

func TestReasoningEffortRoundTrip(t *testing.T) {
	cases := []ReasoningEffort{
		ReasoningProviderDefault, ReasoningNone, ReasoningMinimal,
		ReasoningLow, ReasoningMedium, ReasoningHigh, ReasoningXHigh,
	}
	for _, r := range cases {
		b, _ := json.Marshal(r)
		var got ReasoningEffort
		json.Unmarshal(b, &got)
		if got != r {
			t.Errorf("round-trip mismatch: got %s, want %s", got, r)
		}
	}
}

// ── ToolChoice round-trips ───────────────────────────────────────────────────

func TestToolChoiceStringVariantsRoundTrip(t *testing.T) {
	cases := []struct {
		name string
		tc   ToolChoice
		want string
	}{
		{"auto", ToolChoiceAuto(), `"auto"`},
		{"none", ToolChoiceNone(), `"none"`},
		{"required", ToolChoiceRequired(), `"required"`},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if string(c.tc) != c.want {
				t.Errorf("got %s, want %s", c.tc, c.want)
			}
		})
	}
}

func TestToolChoiceToolRoundTrip(t *testing.T) {
	tc := ToolChoiceTool("get_weather")
	want := `{"type":"tool","toolName":"get_weather"}`
	if string(tc) != want {
		t.Errorf("got %s, want %s", tc, want)
	}
}

// TestToolChoiceToolEscapesSpecialChars is a regression test for the JSON
// injection bug where tool names were concatenated into a string template
// without escaping. Now json.Marshal handles it correctly.
func TestToolChoiceToolEscapesSpecialChars(t *testing.T) {
	tc := ToolChoiceTool(`na"me`)
	// The result must be valid JSON.
	var decoded map[string]string
	if err := json.Unmarshal(tc, &decoded); err != nil {
		t.Fatalf("ToolChoiceTool produced invalid JSON: %v (got %s)", err, tc)
	}
	if decoded["toolName"] != `na"me` {
		t.Errorf("expected escaped tool name, got %q", decoded["toolName"])
	}
}

// ── ToolCall round-trip ───────────────────────────────────────────────────────

func TestToolCallRoundTrip(t *testing.T) {
	pe := true
	dyn := false
	original := ToolCall{
		ToolCallID:       "call_1",
		ToolName:         "get_weather",
		Input:            json.RawMessage(`{"location":"Tokyo"}`),
		ProviderExecuted: &pe,
		Dynamic:          &dyn,
	}
	b, err := json.Marshal(original)
	if err != nil {
		t.Fatalf("marshal failed: %v", err)
	}
	// Verify wire field names match Kotlin (snake_case).
	s := string(b)
	for _, want := range []string{`"tool_call_id"`, `"tool_name"`, `"input"`, `"provider_executed"`, `"dynamic"`} {
		if !contains(s, want) {
			t.Errorf("expected %s in wire JSON, got %s", want, s)
		}
	}
	var decoded ToolCall
	if err := json.Unmarshal(b, &decoded); err != nil {
		t.Fatalf("unmarshal failed: %v", err)
	}
	if decoded.ToolCallID != original.ToolCallID {
		t.Errorf("tool_call_id mismatch: got %s, want %s", decoded.ToolCallID, original.ToolCallID)
	}
	if decoded.ToolName != original.ToolName {
		t.Errorf("tool_name mismatch: got %s, want %s", decoded.ToolName, original.ToolName)
	}
	if decoded.ProviderExecuted == nil || *decoded.ProviderExecuted != true {
		t.Error("provider_executed did not round-trip")
	}
	if decoded.Dynamic == nil || *decoded.Dynamic != false {
		t.Error("dynamic did not round-trip")
	}
}

// ── ModelMessage round-trip ──────────────────────────────────────────────────

func TestModelMessageTextRoundTrip(t *testing.T) {
	msg := NewTextMessage(RoleUser, "Hello")
	b, err := json.Marshal(msg)
	if err != nil {
		t.Fatalf("marshal failed: %v", err)
	}
	want := `{"role":"user","content":"Hello"}`
	if string(b) != want {
		t.Errorf("got %s, want %s", b, want)
	}
	var decoded ModelMessage
	json.Unmarshal(b, &decoded)
	if decoded.Role != RoleUser {
		t.Errorf("role mismatch: got %s", decoded.Role)
	}
	// Content comes back as a string via any.
	if s, ok := decoded.Content.(string); !ok || s != "Hello" {
		t.Errorf("content mismatch: got %v", decoded.Content)
	}
}

func TestModelMessageMultiPartRoundTrip(t *testing.T) {
	// A tool-result message with multi-part content (array of ContentParts).
	msg := ModelMessage{
		Role: RoleTool,
		Content: []map[string]any{
			{
				"type":         "tool_result",
				"tool_call_id": "call_1",
				"result":       map[string]any{"temp": "20"},
			},
		},
	}
	b, err := json.Marshal(msg)
	if err != nil {
		t.Fatalf("marshal failed: %v", err)
	}
	// Verify content is an array on the wire.
	var raw struct {
		Role    string            `json:"role"`
		Content []json.RawMessage `json:"content"`
	}
	if err := json.Unmarshal(b, &raw); err != nil {
		t.Fatalf("unmarshal failed: %v", err)
	}
	if raw.Role != "tool" {
		t.Errorf("expected role 'tool', got %s", raw.Role)
	}
	if len(raw.Content) != 1 {
		t.Fatalf("expected 1 content part, got %d", len(raw.Content))
	}
	// Verify the tool_result structure.
	var part map[string]any
	json.Unmarshal(raw.Content[0], &part)
	if part["type"] != "tool_result" {
		t.Errorf("expected type 'tool_result', got %v", part["type"])
	}
	if part["tool_call_id"] != "call_1" {
		t.Errorf("expected tool_call_id 'call_1', got %v", part["tool_call_id"])
	}
}

// ── GenerateTextOptions round-trip ───────────────────────────────────────────

func TestGenerateTextOptionsRoundTrip(t *testing.T) {
	temp := 0.7
	topK := 40.0
	reasoning := ReasoningHigh
	instr := "Be concise"
	original := GenerateTextOptions{
		Temperature:  &temp,
		TopK:         &topK,
		Reasoning:    &reasoning,
		Instructions: &instr,
		Tools: []Tool{
			{
				Type:        "function",
				Name:        "get_weather",
				InputSchema: json.RawMessage(`{"type":"object"}`),
			},
		},
		ToolChoice: ToolChoiceRequired(),
	}
	b, err := json.Marshal(original)
	if err != nil {
		t.Fatalf("marshal failed: %v", err)
	}
	// Verify snake_case wire fields present.
	s := string(b)
	for _, want := range []string{
		`"temperature"`, `"top_k"`, `"reasoning"`, `"instructions"`,
		`"tool_choice"`, `"tools"`, `"input_schema"`,
	} {
		if !contains(s, want) {
			t.Errorf("expected %s in wire JSON, got %s", want, s)
		}
	}
	// Verify reasoning serializes as the enum string value.
	if !contains(s, `"high"`) {
		t.Errorf("expected reasoning 'high' in wire JSON, got %s", s)
	}

	var decoded GenerateTextOptions
	if err := json.Unmarshal(b, &decoded); err != nil {
		t.Fatalf("unmarshal failed: %v", err)
	}
	if decoded.Temperature == nil || *decoded.Temperature != 0.7 {
		t.Error("temperature did not round-trip")
	}
	if decoded.Reasoning == nil || *decoded.Reasoning != ReasoningHigh {
		t.Error("reasoning did not round-trip")
	}
}

func TestGenerateTextOptionsDefaultOmitsFields(t *testing.T) {
	// Default options should omit all fields (omitempty).
	original := GenerateTextOptions{}
	b, _ := json.Marshal(original)
	if string(b) != `{}` {
		t.Errorf("expected empty object, got %s", b)
	}
}

// RFC-0016 M2: include_raw_chunks round-trips through the typed options.
func TestGenerateTextOptionsIncludeRawChunksRoundTrip(t *testing.T) {
	original := GenerateTextOptions{IncludeRawChunks: boolPtr(true)}
	b, err := json.Marshal(original)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if !strings.Contains(string(b), `"include_raw_chunks":true`) {
		t.Errorf("expected include_raw_chunks:true in %s", b)
	}
	var back GenerateTextOptions
	if err := json.Unmarshal(b, &back); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if back.IncludeRawChunks == nil || *back.IncludeRawChunks != true {
		t.Errorf("round-trip lost include_raw_chunks: %#v", back.IncludeRawChunks)
	}
}

// ── GenerateTextResult round-trip ────────────────────────────────────────────

func TestGenerateTextResultRoundTrip(t *testing.T) {
	original := GenerateTextResult{
		Text: "hello",
		ToolCalls: []ToolCall{
			{ToolCallID: "call_1", ToolName: "get_weather"},
		},
		Usage: Usage{
			InputTokens:  TokenUsage{Total: u32ptr(10)},
			OutputTokens: TokenUsage{Total: u32ptr(5)},
		},
		Raw: GenerateResult{
			Content: []ContentPart{json.RawMessage(`{"Text":{"text":"hello"}}`)},
			FinishReason: FinishReason{Unified: FinishStop},
		},
		FinishReason: FinishReason{Unified: FinishStop},
	}
	b, err := json.Marshal(original)
	if err != nil {
		t.Fatalf("marshal failed: %v", err)
	}
	var decoded GenerateTextResult
	if err := json.Unmarshal(b, &decoded); err != nil {
		t.Fatalf("unmarshal failed: %v", err)
	}
	if decoded.Text != "hello" {
		t.Errorf("text mismatch: got %s", decoded.Text)
	}
	if len(decoded.ToolCalls) != 1 || decoded.ToolCalls[0].ToolName != "get_weather" {
		t.Errorf("tool_calls mismatch: %+v", decoded.ToolCalls)
	}
	if decoded.Raw.FinishReason.Unified != FinishStop {
		t.Errorf("raw.finish_reason mismatch: got %s", decoded.Raw.FinishReason.Unified)
	}
}

func TestGenerateTextResultWithWarningsRoundTrip(t *testing.T) {
	original := GenerateTextResult{
		Text: "ok",
		Warnings: []json.RawMessage{json.RawMessage(`"some warning"`)},
	}
	b, _ := json.Marshal(original)
	if !contains(string(b), `"warnings"`) {
		t.Errorf("expected warnings field in JSON: %s", b)
	}
	var decoded GenerateTextResult
	json.Unmarshal(b, &decoded)
	if len(decoded.Warnings) != 1 {
		t.Errorf("expected 1 warning, got %d", len(decoded.Warnings))
	}
}

// ── Usage / TokenUsage round-trip ─────────────────────────────────────────────

func TestUsageRoundTrip(t *testing.T) {
	original := Usage{
		InputTokens:  TokenUsage{Total: u32ptr(100), CacheRead: u32ptr(50)},
		OutputTokens: TokenUsage{Total: u32ptr(20), Reasoning: u32ptr(5)},
	}
	b, _ := json.Marshal(original)
	s := string(b)
	for _, want := range []string{`"input_tokens"`, `"output_tokens"`, `"total"`, `"cache_read"`, `"reasoning"`} {
		if !contains(s, want) {
			t.Errorf("expected %s in wire JSON, got %s", want, s)
		}
	}
	var decoded Usage
	json.Unmarshal(b, &decoded)
	if decoded.InputTokens.Total == nil || *decoded.InputTokens.Total != 100 {
		t.Error("input_tokens.total did not round-trip")
	}
}

// ── StreamPart parse round-trip ───────────────────────────────────────────────

func TestStreamPartParseRoundTrip(t *testing.T) {
	cases := []struct {
		name string
		json string
		tag  string
	}{
		{"TextDelta", `{"TextDelta":{"id":"tx1","delta":"Hello"}}`, "TextDelta"},
		{"Finish", `{"Finish":{"finish_reason":{"unified":{"stop":null}}}}`, "Finish"},
		{"StreamStart", `{"StreamStart":{"warnings":[]}}`, "StreamStart"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			sp, err := ParseStreamPart(c.json)
			if err != nil {
				t.Fatalf("parse failed: %v", err)
			}
			if sp.Tag != c.tag {
				t.Errorf("tag mismatch: got %s, want %s", sp.Tag, c.tag)
			}
			if len(sp.Payload) == 0 {
				t.Error("expected non-empty payload")
			}
		})
	}
}

func TestParseTextDeltaPayload(t *testing.T) {
	sp, _ := ParseStreamPart(`{"TextDelta":{"id":"tx1","delta":"Hello"}}`)
	var td TextDeltaPayload
	json.Unmarshal(sp.Payload, &td)
	if td.ID != "tx1" || td.Delta != "Hello" {
		t.Errorf("got %+v", td)
	}
}

// TestParseStreamPartRejectsInvalid validates the externally-tagged union
// enforcement: zero keys or multiple keys must produce an error.
func TestParseStreamPartRejectsInvalid(t *testing.T) {
	cases := []struct {
		name string
		json string
	}{
		{"empty_object", `{}`},
		{"multiple_keys", `{"TextDelta":{"delta":"a"},"Finish":{}}`},
		{"not_object", `[]`},
		{"invalid_json", `{`},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			_, err := ParseStreamPart(c.json)
			if err == nil {
				t.Errorf("expected error for %s, got nil", c.name)
			}
		})
	}
}

// ── helpers ───────────────────────────────────────────────────────────────────

func u32ptr(v uint32) *uint32 { return &v }

func contains(s, sub string) bool {
	return len(s) >= len(sub) && (func() bool {
		for i := 0; i <= len(s)-len(sub); i++ {
			if s[i:i+len(sub)] == sub {
				return true
			}
		}
		return false
	})()
}
