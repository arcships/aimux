package aimux

import (
	"encoding/json"
	"strings"
	"testing"
)

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

func TestToolChoiceToolEscapesSpecialChars(t *testing.T) {
	tc := ToolChoiceTool(`na"me`)
	var decoded map[string]string
	if err := json.Unmarshal(tc, &decoded); err != nil {
		t.Fatalf("ToolChoiceTool produced invalid JSON: %v (got %s)", err, tc)
	}
	if decoded["toolName"] != `na"me` {
		t.Errorf("expected escaped tool name, got %q", decoded["toolName"])
	}
}

func TestToolCallRoundTrip(t *testing.T) {
	pe := true
	dyn := false
	original := ToolCall{
		ToolCallID:       "call_1",
		ToolName:         "get_weather",
		Input:            json.RawMessage(`{"location":"Tokyo"}`),
		ProviderExecuted: &pe,
		Dynamic:          &dyn,
		ProviderMetadata: json.RawMessage(`{"openai":{"itemId":"item_1"}}`),
	}
	b, err := json.Marshal(original)
	if err != nil {
		t.Fatalf("marshal failed: %v", err)
	}
	s := string(b)
	for _, want := range []string{`"toolCallId"`, `"toolName"`, `"input"`, `"providerExecuted"`, `"dynamic"`, `"providerMetadata"`} {
		if !strings.Contains(s, want) {
			t.Errorf("expected %s in wire JSON, got %s", want, s)
		}
	}
	var decoded ToolCall
	if err := json.Unmarshal(b, &decoded); err != nil {
		t.Fatalf("unmarshal failed: %v", err)
	}
	if decoded.ToolCallID != original.ToolCallID {
		t.Errorf("toolCallId mismatch: got %s, want %s", decoded.ToolCallID, original.ToolCallID)
	}
	if decoded.ToolName != original.ToolName {
		t.Errorf("toolName mismatch: got %s, want %s", decoded.ToolName, original.ToolName)
	}
	if decoded.ProviderExecuted == nil || *decoded.ProviderExecuted != true {
		t.Error("providerExecuted did not round-trip")
	}
	if decoded.Dynamic == nil || *decoded.Dynamic != false {
		t.Error("dynamic did not round-trip")
	}
	if string(decoded.ProviderMetadata) != string(original.ProviderMetadata) {
		t.Errorf("providerMetadata mismatch: got %s, want %s", decoded.ProviderMetadata, original.ProviderMetadata)
	}
}

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
	if s, ok := decoded.Content.(string); !ok || s != "Hello" {
		t.Errorf("content mismatch: got %v", decoded.Content)
	}
}

func TestModelMessageMultiPartRoundTrip(t *testing.T) {
	msg := ModelMessage{
		Role: RoleTool,
		Content: []map[string]any{
			{
				"type":       "tool-result",
				"toolCallId": "call_1",
				"toolName":   "get_weather",
				"output":     map[string]any{"type": "json", "value": map[string]any{"temp": "20"}},
			},
		},
	}
	b, err := json.Marshal(msg)
	if err != nil {
		t.Fatalf("marshal failed: %v", err)
	}
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
	var part map[string]any
	json.Unmarshal(raw.Content[0], &part)
	if part["type"] != "tool-result" {
		t.Errorf("expected type 'tool-result', got %v", part["type"])
	}
	if part["toolCallId"] != "call_1" {
		t.Errorf("expected toolCallId 'call_1', got %v", part["toolCallId"])
	}
}

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
	s := string(b)
	for _, want := range []string{
		`"temperature"`, `"topK"`, `"reasoning"`, `"instructions"`,
		`"toolChoice"`, `"tools"`, `"inputSchema"`,
	} {
		if !strings.Contains(s, want) {
			t.Errorf("expected %s in wire JSON, got %s", want, s)
		}
	}
	if !strings.Contains(s, `"high"`) {
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

func TestGenerateTextOptionsIncludeRawChunksRoundTrip(t *testing.T) {
	original := GenerateTextOptions{IncludeRawChunks: boolPtr(true)}
	b, err := json.Marshal(original)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if !strings.Contains(string(b), `"includeRawChunks":true`) {
		t.Errorf("expected includeRawChunks:true in %s", b)
	}
	var back GenerateTextOptions
	if err := json.Unmarshal(b, &back); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if back.IncludeRawChunks == nil || *back.IncludeRawChunks != true {
		t.Errorf("round-trip lost includeRawChunks: %#v", back.IncludeRawChunks)
	}
}

func TestGenerateTextResultWithWarningsRoundTrip(t *testing.T) {
	original := GenerateTextResult{
		Text:     "ok",
		Warnings: []json.RawMessage{json.RawMessage(`"some warning"`)},
	}
	b, _ := json.Marshal(original)
	if !strings.Contains(string(b), `"warnings"`) {
		t.Errorf("expected warnings field in JSON: %s", b)
	}
	var decoded GenerateTextResult
	json.Unmarshal(b, &decoded)
	if len(decoded.Warnings) != 1 {
		t.Errorf("expected 1 warning, got %d", len(decoded.Warnings))
	}
}

func TestStreamPartParseRoundTrip(t *testing.T) {
	cases := []struct {
		name string
		json string
		typ  string
	}{
		{"text-delta", `{"type":"text-delta","id":"tx1","text":"Hello"}`, "text-delta"},
		{"finish", `{"type":"finish","finishReason":"stop","totalUsage":{"inputTokens":3,"inputTokenDetails":{},"outputTokenDetails":{}}}`, "finish"},
		{"start-step", `{"type":"start-step","request":{},"warnings":[]}`, "start-step"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			sp, err := ParseStreamPart(c.json)
			if err != nil {
				t.Fatalf("parse failed: %v", err)
			}
			if sp.Type != c.typ {
				t.Errorf("type mismatch: got %s, want %s", sp.Type, c.typ)
			}
			if string(sp.Raw) != c.json {
				t.Errorf("raw mismatch: got %s", sp.Raw)
			}
		})
	}
}

func TestParseTextDeltaPayload(t *testing.T) {
	// The text stream's delta field is "text" (ai stream-text-result.ts:435-440).
	sp, _ := ParseStreamPart(`{"type":"text-delta","id":"tx1","text":"Hello"}`)
	var td TextDeltaPayload
	json.Unmarshal(sp.Raw, &td)
	if td.ID != "tx1" || td.Text != "Hello" {
		t.Errorf("got %+v", td)
	}
	if out, _ := json.Marshal(td); string(out) != `{"id":"tx1","text":"Hello"}` {
		t.Errorf("marshal: %s", out)
	}
}

func TestStreamPartSource(t *testing.T) {
	sp, _ := ParseStreamPart(`{"type":"source","sourceType":"url","id":"s1","url":"https://example.com"}`)
	src, err := sp.Source()
	if err != nil || src.URL == nil || src.URL.URL != "https://example.com" {
		t.Fatalf("source: %+v, %v", src, err)
	}
}

// The finish part's totalUsage is the AI SDK's LanguageModelUsage (ai
// types/usage.ts:10-67): camelCase counts, an unknown count absent.
func TestParseFinishPayload(t *testing.T) {
	sp, _ := ParseStreamPart(`{"type":"finish","finishReason":"tool-calls","rawFinishReason":"tool_use","totalUsage":{"inputTokens":3,"inputTokenDetails":{"cacheReadTokens":1},"outputTokens":2,"outputTokenDetails":{},"totalTokens":5}}`)
	var finish FinishPayload
	if err := json.Unmarshal(sp.Raw, &finish); err != nil {
		t.Fatal(err)
	}
	if finish.FinishReason != "tool-calls" || finish.RawFinishReason == nil || *finish.RawFinishReason != "tool_use" {
		t.Errorf("finish reason: %+v", finish)
	}
	u := finish.TotalUsage
	if u.InputTokens == nil || *u.InputTokens != 3 || u.TotalTokens == nil || *u.TotalTokens != 5 ||
		u.InputTokenDetails.CacheReadTokens == nil || *u.InputTokenDetails.CacheReadTokens != 1 {
		t.Errorf("total usage: %+v", u)
	}
	if out, _ := json.Marshal(u); string(out) != `{"inputTokens":3,"inputTokenDetails":{"cacheReadTokens":1},"outputTokens":2,"outputTokenDetails":{},"totalTokens":5}` {
		t.Errorf("marshal: %s", out)
	}
}

func TestParseStreamPartRejectsInvalid(t *testing.T) {
	cases := []struct {
		name string
		json string
	}{
		{"empty_object", `{}`},
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

func boolPtr(v bool) *bool { return &v }

// Provider options, provider metadata and provider references are JS objects
// upstream (@ai-sdk/provider shared-v4-provider-options.ts:24,
// shared-v4-provider-metadata.ts:24, shared-v4-provider-reference.ts:20), so
// their keys keep insertion order. They go through Rust as ordered maps and
// stay raw JSON in Go, never a sorting Go map.
func TestNamespaceMapsKeepInsertionOrder(t *testing.T) {
	const metadata = `{"zeta":{"z":1,"a":2},"alpha":{},"mid":{}}`
	invalid := `{"type":"tool-call","toolCallId":"call-1","toolName":"weather","input":{"town":"SG"},"dynamic":true,` +
		`"invalid":true,"providerMetadata":` + metadata + `,"error":{"name":"AI_InvalidToolInputError",` +
		`"toolName":"weather","toolInput":"{\"town\":\"SG\"}","cause":"missing city"}}`
	var call ToolCall
	if err := json.Unmarshal([]byte(invalid), &call); err != nil {
		t.Fatalf("decode %s: %v", invalid, err)
	}
	if got := string(call.ProviderMetadata); got != metadata {
		t.Errorf("providerMetadata: got %s, want %s", got, metadata)
	}

	tool, err := json.Marshal(Tool{
		Type:            "function",
		Name:            "weather",
		ProviderOptions: json.RawMessage(`{"zeta":{},"alpha":{}}`),
	})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(tool), `"providerOptions":{"zeta":{},"alpha":{}}`) {
		t.Errorf("tool providerOptions reordered: %s", tool)
	}

	const reference = `{"openai":"file-abc123","anthropic":"file-xyz789","google":"files/1"}`
	var upload UploadFileResult
	if err := json.Unmarshal([]byte(`{"providerReference":`+reference+`}`), &upload); err != nil {
		t.Fatal(err)
	}
	if id, ok := upload.ProviderReference.Get("anthropic"); !ok || id != "file-xyz789" {
		t.Errorf("Get(anthropic) = %q, %v", id, ok)
	}
	if _, ok := upload.ProviderReference.Get("mistral"); ok {
		t.Error("Get(mistral) found an id")
	}
	encoded, _ := json.Marshal(upload)
	if !strings.HasPrefix(string(encoded), `{"providerReference":`+reference) {
		t.Errorf("providerReference reordered: %s", encoded)
	}
	empty, _ := json.Marshal(UploadFileResult{})
	if !strings.HasPrefix(string(empty), `{"providerReference":{}`) {
		t.Errorf("empty providerReference: %s", empty)
	}
}

// supportsDeferredResults is kept when set and omitted when absent
// (provider-utils@5.0.55 src/types/tool.ts:316-326).
func TestProviderToolSupportsDeferredResults(t *testing.T) {
	supported := true
	for _, flag := range []*bool{nil, &supported} {
		provider, err := json.Marshal(Tool{
			Type:                    "provider",
			ID:                      "anthropic.code_execution_20250825",
			Name:                    "code_execution",
			SupportsDeferredResults: flag,
		})
		if err != nil {
			t.Fatal(err)
		}
		want := `{"type":"provider","name":"code_execution","id":"anthropic.code_execution_20250825"}`
		if flag != nil {
			want = `{"type":"provider","name":"code_execution","id":"anthropic.code_execution_20250825","supportsDeferredResults":true}`
		}
		if string(provider) != want {
			t.Errorf("provider tool: got %s, want %s", provider, want)
		}
		var decoded Tool
		if err := json.Unmarshal(provider, &decoded); err != nil {
			t.Fatal(err)
		}
		if (decoded.SupportsDeferredResults == nil) != (flag == nil) {
			t.Errorf("supportsDeferredResults did not round-trip: %s", provider)
		}
	}
}

// ── Standalone type tags (#229 NP2-7) ───────────────────────────────────────

// A result's toolCalls and sources are the AI SDK's TypedToolCall and Source,
// tagged {"type":"tool-call",...} (ai generate-text/tool-call.ts:8-14) and
// {"type":"source","sourceType":...} (provider language-model-v4-source.ts:8,
// :36); generate-text-result.ts:73, :78. A repair function's call is a tagged
// LanguageModelV4ToolCall (language-model-v4-tool-call.ts:7).
func TestStandaloneToolCallsAndSourcesAreTagged(t *testing.T) {
	const wire = `{"text":"","toolCalls":[{"type":"tool-call","toolCallId":"call-1","toolName":"weather","input":{"city":"SG"}}],` +
		`"sources":[{"type":"source","sourceType":"url","id":"s1","url":"https://example.com"},` +
		`{"type":"source","sourceType":"document","id":"s2","mediaType":"application/pdf","title":"Doc"}]}`
	var result GenerateTextResult
	if err := json.Unmarshal([]byte(wire), &result); err != nil {
		t.Fatal(err)
	}
	encoded, err := json.Marshal(result)
	if err != nil {
		t.Fatal(err)
	}
	for _, want := range []string{
		`"toolCalls":[{"type":"tool-call","toolCallId":"call-1","toolName":"weather","input":{"city":"SG"}}]`,
		`"sources":[{"type":"source","sourceType":"url","id":"s1","url":"https://example.com"},` +
			`{"type":"source","sourceType":"document","id":"s2","mediaType":"application/pdf","title":"Doc"}]`,
	} {
		if !strings.Contains(string(encoded), want) {
			t.Errorf("want %s in %s", want, encoded)
		}
	}

	raw, _ := json.Marshal(RawToolCall{ToolCallID: "c", ToolName: "t", Input: "{}"})
	if string(raw) != `{"type":"tool-call","toolCallId":"c","toolName":"t","input":"{}"}` {
		t.Errorf("raw tool call: %s", raw)
	}

	// Missing or mismatched canonical tags are rejected.
	var untagged ToolCall
	if err := json.Unmarshal([]byte(`{"toolCallId":"c","toolName":"t","input":{}}`), &untagged); err == nil {
		t.Error("untagged call accepted")
	}
	var mistagged ToolCall
	if err := json.Unmarshal([]byte(`{"type":"source","toolCallId":"c","toolName":"t"}`), &mistagged); err == nil {
		t.Error("a source-tagged tool call was accepted")
	}
	var source Source
	if err := json.Unmarshal([]byte(`{"type":"tool-call","sourceType":"url","id":"s","url":"u"}`), &source); err == nil {
		t.Error("a tool-call-tagged source was accepted")
	}

}
