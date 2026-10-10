// trace_test.go — RFC-0015 wire-contract tests (pure JSON, no FFI).
//
// Verifies Go can parse the probe wire types exactly as Rust serializes them
// (camelCase fields, hex fingerprints, optional verdicts).

package aimux

import (
	"encoding/json"
	"testing"
)

const traceRecordJSON = `{
  "provider": "openai",
  "model": "gpt-4o",
  "requestId": "req-1",
  "sessionId": "sess-1",
  "callId": "trace-1",
  "sentAtUnixMs": 1785900000000,
  "ttftMs": 42,
  "fingerprint": {
    "bodyHash": "0123456789abcdef0123456789abcdef",
    "lenBytes": 10240,
    "blockSize": 4096,
    "blockHashes": ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"],
    "tokenEstimate": 2560
  },
  "usage": {
    "inputTotal": 2560,
    "inputNoCache": 1536,
    "cacheRead": 1024,
    "cacheWrite": 0,
    "outputTotal": 10,
    "outputText": 10
  },
  "requestCacheHints": {"requestedWrite": true},
  "verdict": {
    "kind": "Trusted",
    "confidence": "High",
    "violated": [],
    "expectedMax": 1024,
    "claimed": 1024,
    "lcpBytes": 4096,
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
		t.Errorf("sessionId mismatch: %v", rec.SessionID)
	}
	if rec.Fingerprint.BodyHash != "0123456789abcdef0123456789abcdef" {
		t.Errorf("bodyHash mismatch: %s", rec.Fingerprint.BodyHash)
	}
	if len(rec.Fingerprint.BlockHashes) != 2 {
		t.Errorf("expected 2 block hashes, got %d", len(rec.Fingerprint.BlockHashes))
	}
	if rec.Usage.CacheRead == nil || *rec.Usage.CacheRead != 1024 {
		t.Errorf("cacheRead mismatch: %v", rec.Usage.CacheRead)
	}
	if rec.RequestCacheHints == nil || !rec.RequestCacheHints.RequestedWrite {
		t.Error("requestCacheHints mismatch")
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
	  "requests": 3, "inputTokensTotal": 7680,
	  "claimedCacheReadTotal": 2048, "claimedCacheWriteTotal": 0,
	  "reportedHitRate": 0.26666666666666666,
	  "clientUpperBoundHitRate": 0.4,
	  "verdictCounts": {"Trusted": 2, "SuspectOverclaim": 1},
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
		t.Errorf("verdictCounts mismatch: %v", stats.VerdictCounts)
	}

	chainJSON := `{
	  "sessionId": "sess-1",
	  "recordIds": ["trace-1", "trace-2"],
	  "prefixStability": 0.8,
	  "breaks": [{
	    "atRecordId": "trace-2", "prevRecordId": "trace-1",
	    "lcpBytes": 1024, "expectedBreak": false, "kind": "Unknown"
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
