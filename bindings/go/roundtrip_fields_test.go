// roundtrip_fields_test.go — input-side reasoning signature echo for the Go
// binding (#135).
//
// Go prompts are raw JSON, so the binding's job is to pass them through the
// FFI into the Rust engine unchanged. This test proves the full pipe for the
// extended-thinking input round-trip: an assistant reasoning part carrying a
// `signature` is echoed as an Anthropic thinking block on the wire when
// thinking is enabled.

package aimux

import (
	"strings"
	"testing"
)

const plainAnthropicResponse = `{"id":"msg_go_rt","type":"message","role":"assistant","model":"claude-sonnet-4-20250514","content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}`

func TestE2E_AnthropicEchoesReasoningSignature(t *testing.T) {
	srv := newMockServer()
	defer srv.Close()
	srv.SetResponse(plainAnthropicResponse)

	m, err := NewAnthropicWithBase("test-key", "claude-sonnet-4-20250514", srv.URL)
	if err != nil {
		t.Fatalf("NewAnthropicWithBase: %v", err)
	}
	defer m.Close()

	prompt := `[
		{"role":"user","content":[{"type":"text","text":"hi"}]},
		{"role":"assistant","content":[
			{"type":"reasoning","text":"pondering","signature":"sig-go-1"},
			{"type":"text","text":"answer"}
		]},
		{"role":"user","content":[{"type":"text","text":"go on"}]}
	]`
	opts := `{"provider_options":{"anthropic":{"thinking":{"type":"enabled","budgetTokens":1024}}}}`

	if _, err := m.GenerateText(prompt, opts); err != nil {
		t.Fatalf("GenerateText: %v", err)
	}

	body := srv.LastRequestBody()
	if !strings.Contains(body, `"type":"thinking"`) {
		t.Errorf("expected a thinking block on the wire, got %s", body)
	}
	if !strings.Contains(body, `"signature":"sig-go-1"`) {
		t.Errorf("expected the prompt signature to be echoed, got %s", body)
	}
}
