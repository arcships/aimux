package aimux

import (
	"encoding/json"
	"math"
	"reflect"
	"testing"
)

func TestHandleIDDecimalWire(t *testing.T) {
	for _, n := range []uint64{0, 1, 1<<53 + 1, math.MaxUint64} {
		raw, err := json.Marshal(HandleID(n))
		if err != nil {
			t.Fatal(err)
		}
		if raw[0] != '"' {
			t.Fatalf("handle encoded as number: %s", raw)
		}
		var got HandleID
		if err := json.Unmarshal(raw, &got); err != nil || uint64(got) != n {
			t.Fatalf("round trip %s: %d, %v", raw, got, err)
		}
	}
	for _, raw := range []string{`1`, `null`, `""`, `"-1"`, `"+1"`, `"01"`, `"1.0"`, `"18446744073709551616"`} {
		id := HandleID(123)
		if err := json.Unmarshal([]byte(raw), &id); err == nil {
			t.Errorf("accepted invalid ID %s", raw)
		}
		if id != 123 {
			t.Errorf("failed decode overwrote ID: %s", raw)
		}
	}
}

func TestWireJSONValidation(t *testing.T) {
	for _, raw := range [][]byte{nil, []byte(`{`), []byte(`{} null`), []byte(`{} invalid`), []byte(`"\ud800"`), []byte(`"\udfff"`), []byte{'"', 0xff, '"'}} {
		if _, err := encodeParams("test.echo", json.RawMessage(raw)); err == nil && raw != nil {
			t.Errorf("accepted %q", raw)
		}
	}
	for _, op := range []string{"", "bad\x00op", "\xff"} {
		if _, err := encodeParams(op, struct{}{}); err == nil {
			t.Errorf("accepted op %q", op)
		}
	}
	for _, raw := range []string{`{}`, `null`, `"\ud83d\ude00"`, `{"explicit":null,"text":"你好\u0000"}`} {
		if _, err := encodeParams("test.echo", json.RawMessage(raw)); err != nil {
			t.Errorf("rejected %s: %v", raw, err)
		}
	}
}

func TestDecodedWireRejectsMalformedAndTrailingJSON(t *testing.T) {
	for _, raw := range []string{"{", "{} {}", "{} invalid", `{"message":"\ud800"}`, "{\"message\":\"\xff\"}"} {
		if _, err := decodeError([]byte(raw)); err == nil {
			t.Errorf("decoded invalid error envelope %q", raw)
		}
		if _, err := validateResult(json.RawMessage(raw), nil); err == nil {
			t.Errorf("accepted invalid raw result %q", raw)
		}
	}
	const valid = "{\"message\":\"ok\"} \n\t"
	if e, err := decodeError([]byte(valid)); err != nil || e.Envelope.Message != "ok" {
		t.Fatalf("decodeError with trailing whitespace = %v, %v", e, err)
	}
}

func roundTripDTO[T any](t *testing.T, raw string) {
	t.Helper()
	var original T
	if err := json.Unmarshal([]byte(raw), &original); err != nil {
		t.Fatal(err)
	}
	encoded, err := json.Marshal(original)
	if err != nil {
		t.Fatal(err)
	}
	var again T
	if err := json.Unmarshal(encoded, &again); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(original, again) {
		t.Fatalf("typed round trip changed value: %s", encoded)
	}
	// Check every supplied field survives too, including nested JSON nulls.
	var want, got any
	json.Unmarshal([]byte(raw), &want)
	json.Unmarshal(encoded, &got)
	if !reflect.DeepEqual(want, got) {
		t.Fatalf("wire round trip changed fields:\nwant %s\ngot %s", raw, encoded)
	}
}

func TestAllModalitiesTypedRoundTrip(t *testing.T) {
	t.Run("text", func(t *testing.T) {
		roundTripDTO[GenerateTextOptions](t, `{"output":{"type":"json"},"temperature":0,"providerOptions":{"z":{"n":null},"a":{}}}`)
	})
	t.Run("embedding", func(t *testing.T) {
		roundTripDTO[EmbeddingResult](t, `{"embeddings":[[0.5,0.25],[1,-1]],"usage":{"tokens":2},"warnings":[]}`)
	})
	t.Run("image", func(t *testing.T) {
		roundTripDTO[ImageResult](t, `{"images":["AAEC"],"response":{},"warnings":[]}`)
	})
	t.Run("speech", func(t *testing.T) {
		roundTripDTO[SpeechResult](t, `{"audio":"AAEC","response":{"body":null},"providerMetadata":{},"warnings":[],"usage":null}`)
	})
	t.Run("transcription", func(t *testing.T) {
		roundTripDTO[TranscriptionResult](t, `{"text":"hello","segments":[{"text":"hi","startSecond":0,"endSecond":1}],"response":{},"warnings":[]}`)
	})
	t.Run("reranking", func(t *testing.T) {
		roundTripDTO[RerankingResult](t, `{"ranking":[{"index":0,"relevanceScore":0.5}],"warnings":[]}`)
	})
	t.Run("video", func(t *testing.T) {
		roundTripDTO[VideoResult](t, `{"videos":[{"type":"binary","data":"AAEC","mediaType":"video/mp4"}],"response":{},"warnings":[]}`)
	})
	t.Run("search", func(t *testing.T) {
		roundTripDTO[SearchResult](t, `{"results":[{"title":"found","url":"https://example.com","score":0}],"warnings":[]}`)
	})
	t.Run("files", func(t *testing.T) {
		roundTripDTO[UploadFileResult](t, `{"providerReference":{"z":"f1","a":"f2"},"warnings":[]}`)
	})
}

func TestGeneratedOptionalFieldsAndLargeNumbers(t *testing.T) {
	raw := `{"text":"hello","voice":null,"providerOptions":{"vendor":{"future":18446744073709551615}}}`
	var dto SpeechCallOptions
	if err := json.Unmarshal([]byte(raw), &dto); err != nil {
		t.Fatal(err)
	}
	if dto.Voice != nil {
		t.Fatal("null voice must decode to nil")
	}
	if dto.Language != nil {
		t.Fatal("absent language became present")
	}
	voice := "new voice"
	dto.Voice = &voice
	encoded, err := json.Marshal(dto)
	if err != nil {
		t.Fatal(err)
	}
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(encoded, &fields); err != nil {
		t.Fatal(err)
	}
	if string(fields["voice"]) != `"new voice"` {
		t.Fatalf("typed mutation not encoded: %s", encoded)
	}
	var options map[string]map[string]json.RawMessage
	if err := json.Unmarshal(fields["providerOptions"], &options); err != nil {
		t.Fatal(err)
	}
	if string(options["vendor"]["future"]) != "18446744073709551615" {
		t.Fatal("rounded opaque number")
	}
}

func TestGeneratedRejectsInvalidFieldType(t *testing.T) {
	var original SpeechCallOptions
	if err := json.Unmarshal([]byte(`{"text":4}`), &original); err == nil {
		t.Fatal("accepted invalid text")
	}
}

func TestGeneratedOptionalFieldsOmitNilAndKeepEmpty(t *testing.T) {
	for _, c := range []struct{ input, want string }{
		{`{}`, `{}`},
		{`{"providerOptions":null,"temperature":null,"tools":null}`, `{}`},
		{`{"providerOptions":{},"temperature":0,"tools":[]}`, `{"providerOptions":{},"temperature":0,"tools":[]}`},
	} {
		var options GenerateTextOptions
		if err := json.Unmarshal([]byte(c.input), &options); err != nil {
			t.Fatal(err)
		}
		raw, err := json.Marshal(options)
		if err != nil || string(raw) != c.want {
			t.Fatalf("%s: got %s, %v; want %s", c.input, raw, err, c.want)
		}
	}
}

func TestGeneratedPresenceKeepsNullAndAbsence(t *testing.T) {
	for _, c := range []struct {
		raw           string
		present, null bool
	}{
		{`{}`, false, false},
		{`{"body":null}`, true, true},
		{`{"body":{"n":18446744073709551615}}`, true, false},
	} {
		roundTripDTO[RequestInfo](t, c.raw)
		var request RequestInfo
		if err := json.Unmarshal([]byte(c.raw), &request); err != nil {
			t.Fatal(err)
		}
		if request.Body.Present != c.present || request.Body.Null != c.null {
			t.Fatalf("%s: %+v", c.raw, request.Body)
		}
	}
}

func TestUntaggedObjectBranchSelection(t *testing.T) {
	var result ReplayLiveResult
	raw := `{"answers":{},"usage":{},"warnings":[],"response":{}}`
	if err := json.Unmarshal([]byte(raw), &result); err != nil {
		t.Fatal(err)
	}
	if _, ok := result.Value.(ReplayLiveResultBranchDecideResult); !ok {
		t.Fatalf("selected %T for a decision result", result.Value)
	}
	for _, raw := range []string{`{}`, `null`, `[]`} {
		if err := json.Unmarshal([]byte(raw), &result); err == nil {
			t.Fatalf("accepted invalid result %s", raw)
		}
	}
}

func TestUnionIgnoresUnrelatedSecondaryTag(t *testing.T) {
	for _, extra := range []string{`null`, `123`, `{"ignored":true}`} {
		var part TextStreamPart
		raw := `{"type":"text-delta","id":"i","text":"ok","sourceType":` + extra + `}`
		if err := json.Unmarshal([]byte(raw), &part); err != nil {
			t.Fatalf("%s: %v", raw, err)
		}
		delta, ok := part.Value.(TextStreamPartVariantTextDelta)
		if !ok || delta.Text != "ok" {
			t.Fatalf("%s: %+v", raw, part.Value)
		}
	}
}

func TestClosedParentsKeepNestedOpenStructsAndClosedVariantsRejectUnknown(t *testing.T) {
	var outputReq OpenAiOutputRequest
	if err := json.Unmarshal([]byte(`{"metadataNamespace":"openai","result":{"steps":[],"totalUsage":{"inputTokens":1,"outputTokens":2,"totalTokens":3,"inputTokenDetails":{},"outputTokenDetails":{}},"futureField":true}}`), &outputReq); err != nil {
		t.Fatalf("closed OpenAiOutputRequest rejected open field in nested GenerateTextResultInput: %v", err)
	}
	if err := json.Unmarshal([]byte(`{"metadataNamespace":"openai","result":{"steps":[],"totalUsage":{"inputTokens":1,"outputTokens":2,"totalTokens":3,"inputTokenDetails":{},"outputTokenDetails":{}}},"extraParent":1}`), &outputReq); err == nil {
		t.Fatal("closed OpenAiOutputRequest accepted unknown parent field")
	}

	var repairReq ToolRepairApplyRequest
	if err := json.Unmarshal([]byte(`{"reply":{"type":"repaired","toolCall":{"toolCallId":"c1","toolName":"t","input":"{}"}},"toolCall":{"toolCallId":"c1","toolName":"t","input":{},"futureCallField":1}}`), &repairReq); err != nil {
		t.Fatalf("closed ToolRepairApplyRequest rejected open field in nested ToolCallInput: %v", err)
	}

	var videoOpts VideoCallOptions
	if err := json.Unmarshal([]byte(`{"frameImages":[{"frameType":"first_frame","image":{"type":"url","url":"https://example.com/a.png"},"futureFrame":1}]}`), &videoOpts); err != nil {
		t.Fatalf("closed VideoCallOptions rejected open field in nested VideoFrameImage: %v", err)
	}

	var backend BackendConfig
	if err := json.Unmarshal([]byte(`{"type":"ring","capacity":4}`), &backend); err != nil {
		t.Fatalf("valid closed tagged variant rejected: %v", err)
	}
	if err := json.Unmarshal([]byte(`{"type":"ring","capacity":4,"extra":true}`), &backend); err == nil {
		t.Fatal("closed tagged variant BackendConfigVariantRing accepted unknown field")
	}
	var recorder CreateRecorderParams
	if err := json.Unmarshal([]byte(`{"backend":{"type":"jsonl"},"capturePolicy":"full"}`), &recorder); err == nil {
		t.Fatal("closed tagged variant BackendConfigVariantJsonl accepted missing required directory")
	}
	if err := json.Unmarshal([]byte(`{"backend":{"type":"jsonl","directory":null},"capturePolicy":"full"}`), &recorder); err == nil {
		t.Fatal("closed tagged variant BackendConfigVariantJsonl accepted null required directory")
	}
	var frame StreamFrame
	if err := json.Unmarshal([]byte(`{"type":"end","seq":"0","outcome":"success","extra":true}`), &frame); err == nil {
		t.Fatal("closed tagged variant StreamFrameVariantEnd accepted unknown field")
	}
}
