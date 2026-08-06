// trace_test.go — RFC-0015 wire-contract tests (pure JSON, no FFI).
//
// Verifies Go can parse the probe wire types exactly as Rust serializes them
// (snake_case fields, hex fingerprints, optional verdicts).

package aimux

import (
	"encoding/json"
	"testing"
)

const traceRecordJSON = `{
  "provider": "openai",
  "model": "gpt-4o",
  "request_id": "req-1",
  "session_id": "sess-1",
  "call_id": "trace-1",
  "sent_at_unix_ms": 1785900000000,
  "ttft_ms": 42,
  "fingerprint": {
    "body_hash": "0123456789abcdef0123456789abcdef",
    "len_bytes": 10240,
    "block_size": 4096,
    "block_hashes": ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"],
    "token_estimate": 2560
  },
  "usage": {
    "input_total": 2560,
    "input_no_cache": 1536,
    "cache_read": 1024,
    "cache_write": 0,
    "output_total": 10,
    "output_text": 10
  },
  "request_cache_hints": {"requested_write": true},
  "verdict": {
    "kind": "Trusted",
    "confidence": "High",
    "violated": [],
    "expected_max": 1024,
    "claimed": 1024,
    "lcp_bytes": 4096,
    "notes": []
  }
}`

func TestTraceRecordParses(t *testing.T) {
	var rec TraceRecord
	if err := json.Unmarshal([]byte(traceRecordJSON), &rec); err != nil {
		t.Fatalf("failed to parse TraceRecord: %v", err)
	}
	if rec.Provider != "openai" || rec.Model != "gpt-4o" {
		t.Errorf("provider/model mismatch: %s / %s", rec.Provider, rec.Model)
	}
	if rec.SessionID == nil || *rec.SessionID != "sess-1" {
		t.Errorf("session_id mismatch: %v", rec.SessionID)
	}
	if rec.Fingerprint.BodyHash != "0123456789abcdef0123456789abcdef" {
		t.Errorf("body_hash mismatch: %s", rec.Fingerprint.BodyHash)
	}
	if len(rec.Fingerprint.BlockHashes) != 2 {
		t.Errorf("expected 2 block hashes, got %d", len(rec.Fingerprint.BlockHashes))
	}
	if rec.Usage.CacheRead == nil || *rec.Usage.CacheRead != 1024 {
		t.Errorf("cache_read mismatch: %v", rec.Usage.CacheRead)
	}
	if rec.RequestCacheHints == nil || !rec.RequestCacheHints.RequestedWrite {
		t.Error("request_cache_hints mismatch")
	}
	if rec.Verdict == nil {
		t.Fatal("verdict must be present")
	}
	var verdict struct {
		Kind string `json:"kind"`
	}
	if err := json.Unmarshal(rec.Verdict, &verdict); err != nil {
		t.Fatalf("verdict parse: %v", err)
	}
	if verdict.Kind != "Trusted" {
		t.Errorf("verdict kind mismatch: %s", verdict.Kind)
	}
}

func TestTraceStatsAndChainParse(t *testing.T) {
	statsJSON := `{
	  "provider": "openai", "model": "gpt-4o",
	  "requests": 3, "input_tokens_total": 7680,
	  "claimed_cache_read_total": 2048, "claimed_cache_write_total": 0,
	  "reported_hit_rate": 0.26666666666666666,
	  "client_upper_bound_hit_rate": 0.4,
	  "verdict_counts": {"Trusted": 2, "SuspectOverclaim": 1},
	  "errors": 0
	}`
	var stats TraceStats
	if err := json.Unmarshal([]byte(statsJSON), &stats); err != nil {
		t.Fatalf("stats parse: %v", err)
	}
	if stats.Requests != 3 || stats.ClaimedCacheReadTotal != 2048 {
		t.Errorf("stats mismatch: %+v", stats)
	}
	if stats.VerdictCounts["Trusted"] != 2 {
		t.Errorf("verdict_counts mismatch: %v", stats.VerdictCounts)
	}

	chainJSON := `{
	  "session_id": "sess-1",
	  "record_ids": ["trace-1", "trace-2"],
	  "prefix_stability": 0.8,
	  "breaks": [{
	    "at_record_id": "trace-2", "prev_record_id": "trace-1",
	    "lcp_bytes": 1024, "expected_break": false, "kind": "Unknown"
	  }]
	}`
	var chain SessionChainView
	if err := json.Unmarshal([]byte(chainJSON), &chain); err != nil {
		t.Fatalf("chain parse: %v", err)
	}
	if len(chain.RecordIDs) != 2 || chain.PrefixStability != 0.8 {
		t.Errorf("chain mismatch: %+v", chain)
	}
	if len(chain.Breaks) != 1 || chain.Breaks[0].Kind != "Unknown" {
		t.Errorf("breaks mismatch: %+v", chain.Breaks)
	}
}
