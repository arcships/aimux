package aimux

import (
	"encoding/json"
	"testing"
)

func TestToolChoiceStringVariantsRoundTrip(t *testing.T) {
	for _, raw := range []string{`"auto"`, `"none"`, `"required"`} {
		roundTripDTO[ToolChoice](t, raw)
	}
}

func TestToolChoiceToolRoundTrip(t *testing.T) {
	roundTripDTO[ToolChoice](t, `{"type":"tool","toolName":"get_weather"}`)
}

func TestToolChoiceToolEscapesSpecialChars(t *testing.T) {
	roundTripDTO[ToolChoice](t, `{"type":"tool","toolName":"na\"me"}`)
}

func TestToolCallRoundTrip(t *testing.T) {
	roundTripDTO[TypedToolCall](t, `{"type":"tool-call","toolCallId":"call_1","toolName":"get_weather","input":{"location":"Tokyo"},"providerExecuted":true,"dynamic":false,"providerMetadata":{"openai":{"itemId":"item_1"}}}`)
}

func TestModelMessageTextRoundTrip(t *testing.T) {
	roundTripDTO[ModelMessage](t, `{"role":"user","content":"Hello"}`)
}

func TestModelMessageMultiPartRoundTrip(t *testing.T) {
	roundTripDTO[ModelMessage](t, `{"role":"tool","content":[{"type":"tool-result","toolCallId":"call_1","toolName":"get_weather","output":{"type":"json","value":{"temp":"20"}}}]}`)
}

func TestGenerateTextOptionsRoundTrip(t *testing.T) {
	roundTripDTO[GenerateTextOptions](t, `{"temperature":0.7,"topK":40,"reasoning":"high","instructions":"Be concise","tools":[{"type":"function","name":"get_weather","inputSchema":{"type":"object"}}],"toolChoice":"required"}`)
}

func TestGenerateTextOptionsDefaultOmitsFields(t *testing.T) {
	raw, err := json.Marshal(GenerateTextOptions{})
	if err != nil || string(raw) != `{}` {
		t.Fatalf("default options: %s, %v", raw, err)
	}
}

func TestGenerateTextOptionsIncludeRawChunksRoundTrip(t *testing.T) {
	include := true
	original := GenerateTextOptions{IncludeRawChunks: &include}
	raw, err := json.Marshal(original)
	if err != nil {
		t.Fatal(err)
	}
	var back GenerateTextOptions
	if err := json.Unmarshal(raw, &back); err != nil {
		t.Fatal(err)
	}
	if back.IncludeRawChunks == nil || !*back.IncludeRawChunks {
		t.Fatal("lost includeRawChunks")
	}
}

func TestStreamPartParseRoundTrip(t *testing.T) {
	for _, raw := range []string{
		`{"type":"text-delta","id":"tx1","text":"Hello"}`,
		`{"type":"finish","finishReason":"stop","totalUsage":{"inputTokens":3,"inputTokenDetails":{},"outputTokenDetails":{}}}`,
		`{"type":"start-step","request":{},"warnings":[]}`,
		`{"type":"source","sourceType":"url","id":"s1","url":"https://example.com"}`,
	} {
		roundTripDTO[TextStreamPart](t, raw)
	}
}

func TestParseStreamPartRejectsInvalid(t *testing.T) {
	for _, raw := range []string{`{}`, `[]`, `{`, `{"type":"future"}`, `{"type":"text-delta","text":4}`} {
		var part TextStreamPart
		if err := json.Unmarshal([]byte(raw), &part); err == nil {
			t.Errorf("accepted %s", raw)
		}
	}
}

func TestNamespaceMapsKeepValues(t *testing.T) {
	roundTripDTO[TypedToolCall](t, `{"type":"tool-call","toolCallId":"call-1","toolName":"weather","input":{"town":"SG"},"dynamic":true,"invalid":true,"providerMetadata":{"zeta":{"z":1,"a":2},"alpha":{},"mid":{}},"error":{"name":"AI_InvalidToolInputError","toolName":"weather","toolInput":"{\"town\":\"SG\"}","cause":{"message":"missing city"}}}`)
	roundTripDTO[UploadFileResult](t, `{"providerReference":{"openai":"file-abc123","anthropic":"file-xyz789","google":"files/1"},"warnings":[]}`)
}

func TestStandaloneToolCallsAndSourcesAreTagged(t *testing.T) {
	roundTripDTO[TypedToolCall](t, `{"type":"tool-call","toolCallId":"call-1","toolName":"weather","input":{"city":"SG"}}`)
	roundTripDTO[TypedSource](t, `{"type":"source","sourceType":"url","id":"s1","url":"https://example.com"}`)
	roundTripDTO[TypedSource](t, `{"type":"source","sourceType":"document","id":"s2","mediaType":"application/pdf","title":"Doc"}`)
	roundTripDTO[TaggedRawToolCall](t, `{"type":"tool-call","toolCallId":"c","toolName":"t","input":"{}"}`)
}

func TestParseTextDeltaPayload(t *testing.T) {
	var part TextStreamPart
	if err := json.Unmarshal([]byte(`{"type":"text-delta","id":"tx1","text":"Hello"}`), &part); err != nil {
		t.Fatal(err)
	}
	delta, ok := part.Value.(TextStreamPartVariantTextDelta)
	if !ok || delta.ID != "tx1" || delta.Text != "Hello" {
		t.Fatalf("text delta: %+v", part.Value)
	}
	delta.Text = "Changed"
	part.Value = delta
	raw, err := json.Marshal(part)
	if err != nil {
		t.Fatal(err)
	}
	var again TextStreamPart
	if err := json.Unmarshal(raw, &again); err != nil {
		t.Fatal(err)
	}
	if again.Value.(TextStreamPartVariantTextDelta).Text != "Changed" {
		t.Fatal("typed variant mutation was not encoded")
	}
}

func TestStreamPartSource(t *testing.T) {
	var part TextStreamPart
	if err := json.Unmarshal([]byte(`{"type":"source","sourceType":"url","id":"s1","url":"https://example.com"}`), &part); err != nil {
		t.Fatal(err)
	}
	source, ok := part.Value.(TextStreamPartVariantSourceURL)
	if !ok || source.URL != "https://example.com" {
		t.Fatalf("source: %+v", part.Value)
	}
}

func TestParseFinishPayload(t *testing.T) {
	const raw = `{"type":"finish","finishReason":"tool-calls","rawFinishReason":"tool_use","totalUsage":{"inputTokens":3,"inputTokenDetails":{"cacheReadTokens":1},"outputTokens":2,"outputTokenDetails":{},"totalTokens":5}}`
	roundTripDTO[TextStreamPart](t, raw)
	var part TextStreamPart
	if err := json.Unmarshal([]byte(raw), &part); err != nil {
		t.Fatal(err)
	}
	finish := part.Value.(TextStreamPartVariantFinish)
	if finish.FinishReason != FinishReasonUnifiedToolCalls || *finish.RawFinishReason != "tool_use" ||
		*finish.TotalUsage.InputTokens != 3 || *finish.TotalUsage.TotalTokens != 5 ||
		*finish.TotalUsage.InputTokenDetails.CacheReadTokens != 1 {
		t.Fatalf("finish: %+v", finish)
	}
}

func TestGenerateTextResultRejectsEmptySteps(t *testing.T) {
	raw := []byte(`{"steps":[],"totalUsage":{"inputTokenDetails":{},"outputTokenDetails":{}}}`)
	var result GenerateTextResult
	if err := json.Unmarshal(raw, &result); err == nil {
		t.Fatal("decoded a text result without any steps")
	}
}

func TestGenerateTextResultWithWarningsRoundTrip(t *testing.T) {
	const facts = `{
      "steps": [{
        "callId":"c1","stepNumber":0,"content":[{"type":"text","text":"ok"}],
        "model":{"modelId":"model","provider":"openai"},"finishReason":"stop",
        "usage":{"inputTokenDetails":{},"outputTokenDetails":{}},
        "performance":{"effectiveOutputTokensPerSecond":0,"effectiveTotalTokensPerSecond":0,
                       "responseTimeMs":0,"stepTimeMs":0,"toolExecutionMs":{}},
        "warnings":[{"type":"other","message":"some warning"}],"request":{},
        "response":{"id":"r1","timestamp":"2026-10-16T00:00:00Z","modelId":"model","messages":[]}
      }],
      "totalUsage":{"inputTokenDetails":{},"outputTokenDetails":{}}
    }`
	roundTripDTO[GenerateTextResult](t, facts)
	var result GenerateTextResult
	if err := json.Unmarshal([]byte(facts), &result); err != nil {
		t.Fatal(err)
	}
	if result.Text() != "ok" || string(result.Output()) != `"ok"` || len(result.Warnings()) != 1 {
		t.Fatalf("unexpected derived result: %q, %s, %v", result.Text(), result.Output(), result.Warnings())
	}

	var parsedNull GenerateTextResult
	withNull := facts[:len(facts)-1] + `,"output":null}`
	if err := json.Unmarshal([]byte(withNull), &parsedNull); err != nil {
		t.Fatal(err)
	}
	if string(parsedNull.Output()) != "null" {
		t.Fatalf("expected explicit null output, got %q", parsedNull.Output())
	}

	unparsed := facts[:len(facts)-1] + `,"outputParsed":false}`
	roundTripDTO[GenerateTextResult](t, unparsed)
	var notParsed GenerateTextResult
	if err := json.Unmarshal([]byte(unparsed), &notParsed); err != nil {
		t.Fatal(err)
	}
	if notParsed.Output() != nil {
		t.Fatalf("expected nil output when outputParsed is false, got %q", notParsed.Output())
	}

	contradictory := facts[:len(facts)-1] + `,"output":null,"outputParsed":false}`
	var bad GenerateTextResult
	if err := json.Unmarshal([]byte(contradictory), &bad); err == nil {
		t.Fatal("decoded a text result with both output and outputParsed=false")
	}
}

// Provider-tool deferred-result support follows the Rust wire contract.
func TestProviderToolSupportsDeferredResults(t *testing.T) {
	roundTripDTO[Tool](t, `{"type":"provider","args":{},"id":"anthropic.code_execution_20250825","name":"code_execution"}`)
	roundTripDTO[Tool](t, `{"type":"provider","args":{},"id":"anthropic.code_execution_20250825","name":"code_execution","supportsDeferredResults":true}`)
}
