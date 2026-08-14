// Contract tests for the Go binding — validates wire format consistency
// against the shared JSON fixture used by all language bindings.
//
// The fixture (contract-tests/fixtures/wire-format.json) contains canonical
// JSON wire-format examples for ToolChoice, StreamPart, GenerateTextOptions,
// Role, ModelMessage, FinishReasonUnified, and ReasoningEffort. This test
// ensures Go parses them identically to Rust, Node, Kotlin, Swift, and Python.
//
// Run from the repo root:
//   cd bindings/go && go test -run TestWireFormat -v

package aimux

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

// fixtureCase is a single entry in the wire-format.json fixture.
type fixtureCase struct {
	Name        string          `json:"name"`
	Type        string          `json:"type"`
	RustValue   string          `json:"rust_value,omitempty"`
	JSON        json.RawMessage `json:"json"`
	Description string          `json:"description"`
}

func loadFixture(t *testing.T) []fixtureCase {
	t.Helper()
	path := filepath.Join("..", "..", "contract-tests", "fixtures", "wire-format.json")
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("failed to read fixture: %v", err)
	}
	var cases []fixtureCase
	if err := json.Unmarshal(data, &cases); err != nil {
		t.Fatalf("failed to parse fixture: %v", err)
	}
	return cases
}

// TestWireFormatConsistency loads the shared fixture and verifies Go can
// decode each wire-format example, ensuring cross-language consistency.
func TestWireFormatConsistency(t *testing.T) {
	cases := loadFixture(t)

	for _, tc := range cases {
		t.Run(tc.Name, func(t *testing.T) {
			// The fixture stores the JSON wire format as a JSON string value
			// (doubly encoded). Decode the outer string to get the actual
			// wire format bytes, then parse those.
			var wireJSON string
			if err := json.Unmarshal(tc.JSON, &wireJSON); err != nil {
				t.Fatalf("failed to extract wire format from fixture: %v", err)
			}

			switch tc.Type {
			case "ToolChoice":
				// ToolChoice is polymorphic on the wire: bare string ("auto")
				// or tagged object ({"type":"tool",...}). Verify raw round-trip.
				var tc2 json.RawMessage
				if err := json.Unmarshal([]byte(wireJSON), &tc2); err != nil {
					t.Fatalf("failed to unmarshal ToolChoice %s: %v", wireJSON, err)
				}
				if string(tc2) != wireJSON {
					t.Errorf("round-trip mismatch: got %s, want %s", tc2, wireJSON)
				}

			case "StreamPart":
				// StreamPart is externally-tagged JSON; verify we can parse
				// the tag and payload.
				sp, err := ParseStreamPart(wireJSON)
				if err != nil {
					t.Fatalf("failed to parse StreamPart: %v", err)
				}
				if sp.Tag == "" {
					t.Error("expected non-empty stream part tag")
				}
				// Verify the tag matches the fixture name's prefix.
				expectedTags := map[string]string{
					"stream_part_text_delta":   "TextDelta",
					"stream_part_finish":       "Finish",
					"stream_part_stream_start": "StreamStart",
					"stream_part_raw":          "Raw",
				}
				if expected, ok := expectedTags[tc.Name]; ok {
					if sp.Tag != expected {
						t.Errorf("expected tag %q, got %q", expected, sp.Tag)
					}
				}

			case "GenerateTextOptions":
				// Default options: all fields null. Verify we can decode.
				var opts GenerateTextOptions
				if err := json.Unmarshal([]byte(wireJSON), &opts); err != nil {
					t.Fatalf("failed to unmarshal GenerateTextOptions: %v", err)
				}
				// Per-fixture expectations. The emptiness checks belong to the
				// default fixture alone — asserting them for every case is what
				// kept the numeric fields untested while they were all null.
				switch tc.Name {
				case "generate_text_options_default":
					if opts.MaxOutputTokens != nil {
						t.Error("expected nil MaxOutputTokens")
					}
					if opts.Temperature != nil {
						t.Error("expected nil Temperature")
					}

				case "generate_text_options_with_session_id":
					if opts.SessionID == nil || *opts.SessionID != "sess-1" {
						t.Errorf("expected session_id \"sess-1\", got %v", opts.SessionID)
					}

				case "generate_text_options_include_raw_chunks_true":
					// RFC-0016 M2 true-case: include_raw_chunks round-trips.
					if opts.IncludeRawChunks == nil || *opts.IncludeRawChunks != true {
						t.Errorf("expected IncludeRawChunks=true, got %v", opts.IncludeRawChunks)
					}

				case "generate_text_options_numeric_types":
					// top_k is fractional on purpose: Go declares *float64, so
					// the fraction must survive. An integer type would truncate
					// 40.5 to 40 and still pass a field-name-only check.
					if opts.TopK == nil || *opts.TopK != 40.5 {
						t.Errorf("expected TopK=40.5, got %v", opts.TopK)
					}
					if opts.FrequencyPenalty == nil || *opts.FrequencyPenalty != -0.5 {
						t.Errorf("expected FrequencyPenalty=-0.5, got %v", opts.FrequencyPenalty)
					}
					if opts.MaxOutputTokens == nil || *opts.MaxOutputTokens != 256 {
						t.Errorf("expected MaxOutputTokens=256, got %v", opts.MaxOutputTokens)
					}
					if opts.Seed == nil || *opts.Seed != 42 {
						t.Errorf("expected Seed=42, got %v", opts.Seed)
					}
					if opts.MaxRetries == nil || *opts.MaxRetries != 3 {
						t.Errorf("expected MaxRetries=3, got %v", opts.MaxRetries)
					}
				}

			case "Role":
				var r Role
				if err := json.Unmarshal([]byte(wireJSON), &r); err != nil {
					t.Fatalf("failed to unmarshal Role: %v", err)
				}
				reencoded, _ := json.Marshal(r)
				if string(reencoded) != wireJSON {
					t.Errorf("round-trip mismatch: got %s, want %s", reencoded, wireJSON)
				}

			case "ModelMessage":
				var msg ModelMessage
				if err := json.Unmarshal([]byte(wireJSON), &msg); err != nil {
					t.Fatalf("failed to unmarshal ModelMessage: %v", err)
				}
				if msg.Role == "" {
					t.Error("expected non-empty role")
				}
				if msg.Content == "" {
					t.Error("expected non-empty content")
				}
				// Round-trip: re-encode and verify structure.
				reencoded, _ := json.Marshal(msg)
				if string(reencoded) != wireJSON {
					t.Errorf("round-trip mismatch: got %s, want %s", reencoded, wireJSON)
				}

			case "FinishReasonUnified":
				var fr FinishReasonUnified
				if err := json.Unmarshal([]byte(wireJSON), &fr); err != nil {
					t.Fatalf("failed to unmarshal FinishReasonUnified: %v", err)
				}
				reencoded, _ := json.Marshal(fr)
				if string(reencoded) != wireJSON {
					t.Errorf("round-trip mismatch: got %s, want %s", reencoded, wireJSON)
				}

			case "ReasoningEffort":
				var re ReasoningEffort
				if err := json.Unmarshal([]byte(wireJSON), &re); err != nil {
					t.Fatalf("failed to unmarshal ReasoningEffort: %v", err)
				}
				reencoded, _ := json.Marshal(re)
				if string(reencoded) != wireJSON {
					t.Errorf("round-trip mismatch: got %s, want %s", reencoded, wireJSON)
				}

			case "TimeoutConfiguration":
				var tc3 TimeoutConfiguration
				if err := json.Unmarshal([]byte(wireJSON), &tc3); err != nil {
					t.Fatalf("failed to unmarshal TimeoutConfiguration: %v", err)
				}
				if tc3.TotalMs == nil || *tc3.TotalMs != 5000 {
					t.Error("expected TotalMs=5000")
				}
				if tc3.FirstChunkMs == nil || *tc3.FirstChunkMs != 1000 {
					t.Error("expected FirstChunkMs=1000")
				}
				if tc3.ChunkMs == nil || *tc3.ChunkMs != 500 {
					t.Error("expected ChunkMs=500")
				}
				reencoded, _ := json.Marshal(tc3)
				if string(reencoded) != wireJSON {
					t.Errorf("round-trip mismatch: got %s, want %s", reencoded, wireJSON)
				}

			case "GenerateContent":
				// Go keeps result content parts as raw JSON by design
				// (ContentPart = json.RawMessage), so there is no typed
				// GenerateContent to decode into. Verify the wire shape
				// survives Go's JSON layer byte-for-byte — the same check
				// ToolChoice gets, and for the same reason.
				var gc json.RawMessage
				if err := json.Unmarshal([]byte(wireJSON), &gc); err != nil {
					t.Fatalf("failed to unmarshal GenerateContent %s: %v", wireJSON, err)
				}
				if string(gc) != wireJSON {
					t.Errorf("round-trip mismatch: got %s, want %s", gc, wireJSON)
				}

			default:
				t.Fatalf("unsupported fixture type: %s (add Go support for this wire type)", tc.Type)
			}
		})
	}
}
