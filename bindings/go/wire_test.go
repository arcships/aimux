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
	for _, raw := range [][]byte{nil, []byte(`{`), []byte(`"\ud800"`), []byte(`"\udfff"`), []byte{'"', 0xff, '"'}} {
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
		roundTripDTO[EmbeddingResult](t, `{"embeddings":[[0.5,0.25],[1,-1]],"usage":{"tokens":2},"providerMetadata":null}`)
	})
	t.Run("image", func(t *testing.T) {
		roundTripDTO[ImageResult](t, `{"images":["AAEC"],"response":{},"providerMetadata":null}`)
	})
	t.Run("speech", func(t *testing.T) {
		roundTripDTO[SpeechResult](t, `{"audio":"AAEC","response":{"body":null},"providerMetadata":{}}`)
	})
	t.Run("transcription", func(t *testing.T) {
		roundTripDTO[TranscriptionResult](t, `{"text":"hello","segments":[{"text":"hi","startSecond":0,"endSecond":1}],"response":{},"providerMetadata":null}`)
	})
	t.Run("reranking", func(t *testing.T) {
		roundTripDTO[RerankingResult](t, `{"ranking":[{"index":0,"relevanceScore":0.5}],"providerMetadata":null}`)
	})
	t.Run("video", func(t *testing.T) {
		roundTripDTO[VideoResult](t, `{"videos":[{"type":"binary","data":"AAEC","mediaType":"video/mp4"}],"response":{},"providerMetadata":null}`)
	})
	t.Run("search", func(t *testing.T) {
		roundTripDTO[SearchResult](t, `{"results":[{"title":"found","url":"https://example.com","score":0}],"providerMetadata":null}`)
	})
	t.Run("files", func(t *testing.T) {
		roundTripDTO[UploadFileResult](t, `{"providerReference":{"z":"f1","a":"f2"},"providerMetadata":null}`)
	})
}

func TestWireDTOKeepsNullMissingAndUnknownFields(t *testing.T) {
	// Ordinary *string decodes both null and absence as nil. WireDTO retains
	// the canonical document independently of that typed view.
	raw := `{"text":"hello","voice":null,"providerOptions":{"n":null},"future":18446744073709551615}`
	var dto WireDTO[SpeechCallOptions]
	if err := json.Unmarshal([]byte(raw), &dto); err != nil {
		t.Fatal(err)
	}
	value, err := dto.Value()
	if err != nil || value.Text != "hello" || value.Voice != nil {
		t.Fatalf("typed view: %+v, %v", value, err)
	}
	if field, present := dto.Field("voice"); !present || string(field) != "null" {
		t.Fatalf("explicit null: %s,%v", field, present)
	}
	if field, present := dto.Field("language"); present || field != nil {
		t.Fatalf("absent field: %s,%v", field, present)
	}
	encoded, err := json.Marshal(dto)
	if err != nil || string(encoded) != raw {
		t.Fatalf("lossy typed DTO: %s,%v", encoded, err)
	}
	// Mutating the original native-buffer copy or a typed view cannot change
	// the immutable wire document.
	value.ProviderOptions[0] = '['
	encoded, err = json.Marshal(dto)
	if err != nil || string(encoded) != raw {
		t.Fatalf("typed view aliases wire storage: %s,%v", encoded, err)
	}
}

func TestWireDTOCopiedValueDoesNotAliasOnDecode(t *testing.T) {
	var original WireDTO[SpeechCallOptions]
	if err := json.Unmarshal([]byte(`{"text":"original","voice":null}`), &original); err != nil {
		t.Fatal(err)
	}
	copied := original
	if err := json.Unmarshal([]byte(`{"text":"changed"}`), &copied); err != nil {
		t.Fatal(err)
	}
	raw, err := json.Marshal(original)
	if err != nil || string(raw) != `{"text":"original","voice":null}` {
		t.Fatalf("copied DTO mutated original: %s,%v", raw, err)
	}
}
