package aimux

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// A 429 arrives as CodeAPICall; the classification is Status, and the hint
// rides along in RetryMs.
func TestErrorAs(t *testing.T) {
	inner := &Error{
		Code:    CodeAPICall,
		Message: "API call error: HTTP 429: slow down",
		Status:  429,
		RetryMs: 1000,
	}
	wrapped := fmt.Errorf("call failed: %w", inner)

	var e *Error
	if !errors.As(wrapped, &e) {
		t.Fatal("errors.As(*Error) failed")
	}
	if e.Code != CodeAPICall {
		t.Fatalf("Code: got %v", e.Code)
	}
	if e.Status != 429 || e.RetryMs != 1000 {
		t.Fatalf("Status/RetryMs: got %d / %d", e.Status, e.RetryMs)
	}
	if e.Code.String() != "ApiCall" {
		t.Fatalf("Code.String: %s", e.Code.String())
	}
}

// Auth (401) and model-not-found (404) share CodeAPICall and are distinguished
// apart by Status alone.
func TestAPICallClassification(t *testing.T) {
	for _, c := range []struct {
		status int
		msg    string
	}{
		{401, "API call error: HTTP 401: invalid api key"},
		{404, "API call error: HTTP 404: model not found"},
		{-1, "API call error: connection reset"}, // transport: no response
	} {
		e := &Error{Code: CodeAPICall, Message: c.msg, Status: c.status, RetryMs: -1}
		if got := defaultStatus(e.Code, e.Status); got != c.status {
			t.Errorf("defaultStatus(ApiCall, %d) = %d; want %d", c.status, got, c.status)
		}
		if e.Error() != c.msg {
			t.Errorf("Error() = %q; want %q", e.Error(), c.msg)
		}
	}
}

// An AiMuxError carries its per-code payload in the matching field.
func TestPayloadEngineFailure(t *testing.T) {
	_, err := Provider("no-such-provider", "sk-test-fake-key", "some-model")
	if err == nil {
		t.Fatal("expected error for unknown provider")
	}
	var e *Error
	if !errors.As(err, &e) {
		t.Fatalf("expected *Error, got %T: %v", err, err)
	}
	if e.Code != CodeNoSuchProvider {
		t.Fatalf("Code: got %v, want NoSuchProvider", e.Code)
	}
	if e.ProviderID != "no-such-provider" {
		t.Fatalf("ProviderID: got %q", e.ProviderID)
	}
	if e.ProviderCode != "" || e.ProviderMessage != "" || e.RequestID != "" || e.ResponseBody != "" || e.ModelID != "" || e.ModelType != "" {
		t.Fatalf("payload fields of other codes must be empty: %+v", e)
	}
}

// Use-after-close is the binding's own guard: an ErrClosed-wrapped error,
// never an *Error.
func TestUseAfterCloseIsErrClosed(t *testing.T) {
	m := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	m.Close()

	_, err := m.GenerateText(`"hello"`, "")
	if err == nil {
		t.Fatal("expected error after close")
	}
	if !errors.Is(err, ErrClosed) {
		t.Fatalf("expected ErrClosed, got %T: %v", err, err)
	}
	var e *Error
	if errors.As(err, &e) {
		t.Fatal("use-after-close must not be an *Error")
	}
}

// TestDefaultStatus: TokenExpired is a 401 by contract; every other status is
// the observed one. Nothing is invented for ApiCall — a missing status there
// means no response arrived. A concrete status is never overwritten.
func TestDefaultStatus(t *testing.T) {
	cases := []struct {
		code     Code
		in, want int
	}{
		{CodeTokenExpired, -1, 401},
		{CodeTokenExpired, 401, 401},
		// Observed ApiCall statuses pass through untouched.
		{CodeAPICall, 429, 429},
		{CodeAPICall, 401, 401},
		{CodeAPICall, 404, 404},
		{CodeAPICall, 503, 503},
		// Transport failure: no response, so no status is fabricated.
		{CodeAPICall, -1, -1},
		// Non-HTTP codes keep -1.
		{CodeTimeout, -1, -1},
		{CodeAborted, -1, -1},
		{CodeInvalidArgument, -1, -1},
	}
	for _, c := range cases {
		got := defaultStatus(c.code, c.in)
		if got != c.want {
			t.Errorf("defaultStatus(%s, %d) = %d; want %d", c.code, c.in, got, c.want)
		}
	}
}

// Retryable crosses the ABI as its own field. Two ApiCall failures both report
// Status -1 and disagree about retrying, so Status cannot stand in for it.
func TestRetryableIsNotDerivedFromStatus(t *testing.T) {
	transport := &Error{
		Code:      CodeAPICall,
		Message:   "API call error: connection reset",
		Status:    -1,
		RetryMs:   -1,
		Retryable: true, // request went out
	}
	missingKey := &Error{
		Code:    CodeAPICall,
		Message: "API call error: missing api key",
		Status:  -1,
		RetryMs: -1,
	} // request never went out; Retryable stays false

	if transport.Status != -1 || missingKey.Status != -1 {
		t.Fatalf("both must carry Status -1: got %d / %d", transport.Status, missingKey.Status)
	}
	if !transport.Retryable || missingKey.Retryable {
		t.Fatalf("Retryable: transport=%v missingKey=%v; want true / false",
			transport.Retryable, missingKey.Retryable)
	}
}

// Malformed raw JSON is caught by the binding before the C call: a plain
// error naming the parameter, not an *Error.
func TestInvalidJSONNamesParameter(t *testing.T) {
	err := RegisterProviders("{not json")
	if err == nil {
		t.Fatal("expected error for malformed config")
	}
	if !strings.Contains(err.Error(), "config_json") {
		t.Fatalf("message must name the parameter: %v", err)
	}
	var e *Error
	if errors.As(err, &e) {
		t.Fatalf("C ABI error must not be an *Error")
	}

	m := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	defer m.Close()
	_, err = m.GenerateText("{not json", "")
	if err == nil || !strings.Contains(err.Error(), "prompt_json") {
		t.Fatalf("expected prompt_json error, got %v", err)
	}
	if errors.As(err, &e) {
		t.Fatalf("C ABI error must not be an *Error")
	}
}

// A malformed stream prompt surfaces via Stream.Err, and Close remains
// non-blocking because stream execution never owns a Go lifecycle lock.
func TestStreamTextInvalidJSONReleasesModel(t *testing.T) {
	m := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	st := m.StreamText("{bad", "")
	for range st.Parts() {
	}
	if err := st.Err(); err == nil || !strings.Contains(err.Error(), "prompt_json") {
		t.Fatalf("expected prompt_json error, got %v", err)
	}
	closed := make(chan struct{})
	go func() { m.Close(); close(closed) }()
	select {
	case <-closed:
	case <-time.After(5 * time.Second):
		t.Fatal("Close hung during StreamText")
	}
}

// Required raw-JSON parameters reject "" before the C call; optional ones
// treat it as the default.
func TestRequiredJSONRejectsEmpty(t *testing.T) {
	m := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	defer m.Close()
	if _, err := m.GenerateText("", ""); err == nil || !strings.Contains(err.Error(), "prompt_json: invalid JSON: empty") {
		t.Fatalf("expected empty prompt_json error, got %v", err)
	}
	if err := checkJSON("opts_json", ""); err != nil {
		t.Fatalf("optional param must accept empty: %v", err)
	}
	if _, err := MockReplay("{\"ok\":1}\n{bad\n"); err == nil || !strings.Contains(err.Error(), "recordings_jsonl") {
		t.Fatalf("expected recordings_jsonl error, got %v", err)
	}
}

// RecordingTryFlush: nil with nothing recording. InitRecording into a dir whose
// parent is a regular file fails at init with Code INIT (nothing installed), so
// a subsequent RecordingTryFlush is nil.
func TestRecordingTryFlush(t *testing.T) {
	RecordingStop()
	if err := RecordingTryFlush(); err != nil {
		t.Fatalf("nothing recording: want nil, got %v", err)
	}

	blocker := t.TempDir()
	occupied := filepath.Join(blocker, "occupied")
	if err := os.WriteFile(occupied, []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	err := InitRecording(filepath.Join(occupied, "sub"))
	defer RecordingStop()

	var re *RecordingError
	if !errors.As(err, &re) {
		t.Fatalf("want *RecordingError, got %T (%v)", err, err)
	}
	if re.Code != RecordingErrorInit {
		t.Fatalf("Code = %v; want RecordingErrorInit", re.Code)
	}
	var e *Error
	if errors.As(err, &e) {
		t.Fatalf("recording error must not be an *Error")
	}
	if err := RecordingTryFlush(); err != nil {
		t.Fatalf("failed init installs nothing: want nil, got %v", err)
	}
}

func TestRecordingErrorCodeFromCodeRejectsOutOfRange(t *testing.T) {
	if _, ok := recordingErrorCodeFromC(999); ok {
		t.Fatal("999 is not a Rust RecordingError variant")
	}
	if k, ok := recordingErrorCodeFromC(103); !ok || k != RecordingErrorWriterGone {
		t.Fatalf("4 → %v, %v", k, ok)
	}
}

// The C ABI policy: input a caller can produce never
// crashes the process. Everything below used to reach C — where the failure
// has no public code — and panic. Each case must come back as a plain error:
// not an *Error, not a *RecordingError, and not ErrClosed unless the handle
// really is closed.
func TestFfiFailuresReturnErrorsNotPanics(t *testing.T) {
	m := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	defer m.Close()
	tr, err := m.Trace()
	if err != nil {
		t.Fatal(err)
	}
	defer tr.Close()
	dead := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	dead.Close()

	badUTF8 := string([]byte{0xff, 0xfe})

	for _, tc := range []struct {
		name string
		call func() error
		want error // errors.Is target; nil = a plain error carrying no sentinel
		msg  string
	}{
		{"non-utf8 session id", func() error { _, e := tr.TraceSessionChain(badUTF8); return e }, nil, "session_id: must be valid UTF-8"},
		{"non-utf8 prompt json", func() error { _, e := m.GenerateText(string([]byte{0x22, 0xff, 0x22}), ""); return e }, nil, "prompt_json: must be valid UTF-8"},
		{"non-utf8 model id", func() error { _, e := NewOpenAI("sk-test", badUTF8); return e }, nil, "model_id: must be valid UTF-8"},
		{"non-utf8 embedding api key", func() error { _, e := NewOpenAIEmbedding(badUTF8, "m"); return e }, nil, "api_key: must be valid UTF-8"},
		{"lone high surrogate prompt", func() error { _, e := m.GenerateText(`"\ud800"`, ""); return e }, nil, "prompt_json: invalid JSON"},
		{"lone low surrogate filter", func() error { _, e := tr.TraceAggregate(`{"provider":"\udc00"}`); return e }, nil, "filter_json: invalid JSON"},
		{"embedded NUL session id", func() error { _, e := tr.TraceSessionChain("a\x00b"); return e }, nil, "session_id: must not contain NUL"},
		{"embedded NUL base url", func() error { _, e := NewOpenAIWithBase("k", "m", "http://x\x00y"); return e }, nil, "base_url: must not contain NUL"},
		// State guard runs before argument validation: an untraced model says
		// so even when the filter is also malformed.
		{"untraced trace query", func() error { _, e := m.TraceAggregate("{bad"); return e }, ErrNotTraced, ""},
		{"dead handle", func() error { _, e := dead.GenerateText(`"hi"`, ""); return e }, ErrClosed, ""},
		{"dead handle trace export", func() error { _, e := dead.TraceExportJsonl(); return e }, ErrNotTraced, ""},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var err error
			var panicked any
			func() {
				defer func() { panicked = recover() }()
				err = tc.call()
			}()
			if panicked != nil {
				t.Fatalf("panicked instead of returning an error: %v", panicked)
			}
			if err == nil {
				t.Fatal("expected an error")
			}
			if tc.msg != "" && !strings.Contains(err.Error(), tc.msg) {
				t.Errorf("message = %q; want it to contain %q", err, tc.msg)
			}
			if tc.want != nil {
				if !errors.Is(err, tc.want) {
					t.Fatalf("errors.Is(%v, %v) = false", err, tc.want)
				}
				return
			}
			var ae *Error
			if errors.As(err, &ae) {
				t.Errorf("C ABI failure must not be an *Error: %v", err)
			}
			var re *RecordingError
			if errors.As(err, &re) {
				t.Errorf("C ABI failure must not be a *RecordingError: %v", err)
			}
			if errors.Is(err, ErrClosed) {
				t.Errorf("C ABI failure must not be ErrClosed: %v", err)
			}
		})
	}
}

// json.Valid accepts unpaired \uXXXX surrogate halves; serde_json rejects them
// as a syntax error in C. checkJSON has to reject what serde does.
func TestCheckJSONSurrogatePairing(t *testing.T) {
	for _, tc := range []struct {
		json string
		ok   bool
	}{
		{`{"a":"\ud800"}`, false},       // lone high
		{`{"a":"\udc00"}`, false},       // lone low
		{`{"a":"\ud800\ud800"}`, false}, // high followed by high
		{`{"a":"\ud83d\ude00"}`, true},  // well-formed pair
		{`{"a":"\\ud800"}`, true},       // escaped backslash, not an escape
		{`{"a":"A\ud83d\ude00B"}`, true},
		{`{"a":"plain"}`, true},
	} {
		err := checkJSON("filter_json", tc.json)
		if tc.ok && err != nil {
			t.Errorf("%s: unexpected error %v", tc.json, err)
		}
		if !tc.ok {
			if err == nil {
				t.Errorf("%s: expected a surrogate error", tc.json)
			} else if !strings.Contains(err.Error(), "filter_json: invalid JSON") {
				t.Errorf("%s: message = %q", tc.json, err)
			}
		}
	}
}

// InitLogging has no error channel, so an unusable level must fall back to the
// default instead of taking the process down. The reachable call is
// InitLogging(os.Getenv("AIMUX_LOG_LEVEL")) — os.Getenv returns raw bytes.
func TestInitLoggingNeverPanicsOnBadLevel(t *testing.T) {
	for _, level := range []string{"", "\xff", string([]byte{0xff, 0xfe}), "a\x00b", "warn", "nonsense"} {
		func() {
			defer func() {
				if p := recover(); p != nil {
					t.Errorf("InitLogging(%q) panicked: %v", level, p)
				}
			}()
			InitLogging(level)
		}()
	}
}

// json.Marshal does NOT sanitize everything it emits: it coerces invalid UTF-8
// in a Go string to U+FFFD, but json.RawMessage / jsonObj fields go through
// compact(), which validates syntax only. So a lone surrogate escape or a raw
// non-UTF-8 byte planted in ProviderOptions survives marshalling and would
// reach serde_json / CStr::to_str. Every marshalled C argument is checked.
func TestMarshalledOptionsRejectRawPassThrough(t *testing.T) {
	loneSurrogate := jsonObj(`{"a":"\ud800"}`)
	rawNonUTF8 := jsonObj("{\"a\":\"\xff\"}")

	for _, tc := range []struct {
		name string
		call func() error
		msg  string
	}{
		{"speech opts lone surrogate", func() error {
			_, e := requiredOptsJSON("SpeechModel.Generate", &SpeechCallOptions{Text: "hi", ProviderOptions: loneSurrogate})
			return e
		}, "opts: invalid JSON: unpaired high surrogate \\uD800"},
		{"speech opts raw non-utf8", func() error {
			_, e := requiredOptsJSON("SpeechModel.Generate", &SpeechCallOptions{Text: "hi", ProviderOptions: rawNonUTF8})
			return e
		}, "opts: must be valid UTF-8"},
		{"image opts lone surrogate", func() error {
			_, e := requiredOptsJSON("ImageModel.Generate", &ImageCallOptions{ProviderOptions: loneSurrogate})
			return e
		}, "opts: invalid JSON"},
		{"provider config lone surrogate", func() error {
			_, e := ProviderWithConfig("groq", "k", "m", &ProviderConfig{
				BodyOverrides: map[string]any{"x": json.RawMessage(`"\udc00"`)},
			})
			return e
		}, "config_json: invalid JSON: lone low surrogate \\uDC00"},
		{"transcription session opts lone surrogate", func() error {
			b, e := marshalJSON("opts", &TranscriptionSessionOpts{
				ProviderOptions: map[string]json.RawMessage{"x": json.RawMessage(`"\ud800"`)},
			})
			_ = b
			return e
		}, "opts: invalid JSON"},
		// The claim that holds: a plain Go string field really is sanitized —
		// invalid UTF-8 becomes U+FFFD, NUL becomes a \u0000 escape. No error here.
		{"plain string field is sanitized, not rejected", func() error {
			s, e := marshalJSON("opts", &SpeechCallOptions{Text: "hi\xffthere\x00"})
			if e == nil && !strings.Contains(s, `"hi\ufffdthere\u0000"`) {
				t.Errorf("expected U+FFFD / \\u0000 coercion, got %s", s)
			}
			return e
		}, ""},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var err error
			var panicked any
			func() {
				defer func() { panicked = recover() }()
				err = tc.call()
			}()
			if panicked != nil {
				t.Fatalf("panicked instead of returning an error: %v", panicked)
			}
			if tc.msg == "" {
				if err != nil {
					t.Fatalf("unexpected error: %v", err)
				}
				return
			}
			if err == nil {
				t.Fatal("expected an error")
			}
			if !strings.Contains(err.Error(), tc.msg) {
				t.Errorf("message = %q; want it to contain %q", err, tc.msg)
			}
		})
	}
}

// Embed marshals both arguments; the opts blob is the one with a raw field.
func TestEmbedRejectsRawPassThroughOpts(t *testing.T) {
	m, err := NewOpenAIEmbedding("sk-test-fake-key", "text-embedding-3-small")
	if err != nil {
		t.Fatal(err)
	}
	defer m.Close()

	if _, err := m.Embed([]string{"hi"}, &EmbeddingCallOptions{
		ProviderOptions: jsonObj(`{"a":"\ud800"}`),
	}); err == nil || !strings.Contains(err.Error(), "opts: invalid JSON") {
		t.Fatalf("Embed with a lone surrogate in provider_options: %v", err)
	}
}

func TestCodeFromCRejectsOutOfRange(t *testing.T) {
	for _, bad := range []int{0, 14, 15, 999} {
		if _, ok := codeFromC(bad); ok {
			t.Fatalf("%d is not an AiMuxError variant", bad)
		}
	}
	// 1 is the catch-all Other (it took the deleted UNKNOWN's slot).
	if c, ok := codeFromC(1); !ok || c != CodeOther {
		t.Fatalf("1 → %v, %v; want CodeOther", c, ok)
	}
	if c, ok := codeFromC(11); !ok || c != CodeAPICall {
		t.Fatalf("11 → %v, %v", c, ok)
	}
}
