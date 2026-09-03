package aimux

import (
	"encoding/json"
	"errors"
	"fmt"
	"strconv"
	"strings"
	"unicode/utf8"
)

// Code is the machine-readable Aimux error code. Values match
// aimux-ffi aimux_error_code_t (1..13 and 15..17 = core AiMuxError variants;
// 1 is the catch-all Other; 4 is retired, 14 is reserved).
// A code outside that range is a header/library mismatch and expectAimuxError
// panics rather than inventing an "unknown" variant. Recording failures are
// a different type: see RecordingError.
//
// Every HTTP-shaped failure arrives as CodeAPICall, classified by Status
// (401 auth, 404 model, 429 rate limit; Status -1 = no HTTP response was ever
// observed — a missing API key, an error built without a request, or a
// transport failure — which says nothing about whether a retry would help).
type Code int

const (
	CodeOK                  Code = 0
	CodeOther               Code = 1
	CodeJSONParse           Code = 2
	CodeInvalidResponseData Code = 3
	// 4 is retired (the legacy Tool variant); 14 is reserved.
	CodeInvalidArgument          Code = 5
	CodeInvalidPrompt            Code = 6
	CodeTokenExpired             Code = 7
	CodeUnsupportedFunctionality Code = 8
	CodeNoSuchModel              Code = 9
	CodeNoSuchProvider           Code = 10
	CodeAPICall                  Code = 11
	CodeTimeout                  Code = 12
	CodeAborted                  Code = 13
	CodeNoSuchTool               Code = 15
	CodeInvalidToolInput         Code = 16
	CodeToolCallRepair           Code = 17
)

// String returns the core error_type name (e.g. "ApiCall", "TokenExpired").
// Unmapped codes render as "Code(n)".
func (c Code) String() string {
	switch c {
	case CodeOK:
		return "OK"
	case CodeJSONParse:
		return "JsonParse"
	case CodeInvalidResponseData:
		return "InvalidResponseData"
	case CodeInvalidArgument:
		return "InvalidArgument"
	case CodeInvalidPrompt:
		return "InvalidPrompt"
	case CodeTokenExpired:
		return "TokenExpired"
	case CodeUnsupportedFunctionality:
		return "UnsupportedFunctionality"
	case CodeNoSuchModel:
		return "NoSuchModel"
	case CodeNoSuchProvider:
		return "NoSuchProvider"
	case CodeAPICall:
		return "ApiCall"
	case CodeTimeout:
		return "Timeout"
	case CodeAborted:
		return "Aborted"
	case CodeNoSuchTool:
		return "NoSuchTool"
	case CodeInvalidToolInput:
		return "InvalidToolInput"
	case CodeToolCallRepair:
		return "ToolCallRepair"
	case CodeOther:
		return "Other"
	default:
		return fmt.Sprintf("Code(%d)", int(c))
	}
}

// codeFromC maps a C aimux_error_code_t (1..13, 15..17); false for any other
// value — including the retired 4 and the reserved 14.
func codeFromC(code int) (Code, bool) {
	if code < int(CodeOther) || code > int(CodeToolCallRepair) || code == 4 || code == 14 {
		return 0, false
	}
	return Code(code), true
}

// Error is the structured aimux failure type for Go (openai-go style: one
// struct implementing error, not a class tree).
//
// Transport: returned as the error value of (T, error).
// Inspection:
//
//	var e *aimux.Error
//	if errors.As(err, &e) {
//	    // e.Code, e.Status, e.RetryMs, e.Retryable, e.Message
//	    // e.Code == aimux.CodeAPICall && e.Status == 429 → rate limited
//	}
//
// Fields mirror the aimux-ffi aimux_error_* getters / core helpers:
//   - Code: error code (ApiCall, TokenExpired, …)
//   - Status: HTTP status, or -1 — the classification for CodeAPICall
//     (401 auth, 404 model, 429 rate limit)
//   - RetryMs: rate-limit hint, or -1 (0 = retry now)
//   - Retryable: the core's retry verdict, carried across the ABI
//   - Message: human-readable text
//   - ProviderCode / ProviderMessage / RequestID / ResponseBody: CodeAPICall payload
//   - ModelID / ModelType: CodeNoSuchModel payload
//   - ProviderID: CodeNoSuchProvider payload
//   - ToolName / AvailableTools: CodeNoSuchTool payload (ToolName is shared
//     with CodeInvalidToolInput)
//   - ToolInput: CodeInvalidToolInput payload
//   - OriginalError: CodeToolCallRepair payload
type Error struct {
	Code    Code
	Message string
	Status  int   // HTTP status or -1
	RetryMs int64 // retry hint or -1; 0 = retry immediately
	// Retryable is the core's verdict on whether retrying may help. It
	// rides the ABI as its own field and is never derived from Status: a
	// transport failure (request went out, connection reset) is retryable
	// and a missing API key (request never went out) is not, yet both
	// report Status -1.
	Retryable bool
	// Per-code payload, mirroring aimux-core's AiMuxError variants. Each
	// field is set only under the Code named here and empty otherwise.
	ProviderCode    string // CodeAPICall: provider's own error code, e.g. "insufficient_quota"
	ProviderMessage string // CodeAPICall: the failure's own text without the composed prefix Message carries, e.g. "slow down"
	RequestID       string // CodeAPICall: provider request id
	ResponseBody    string // CodeAPICall: raw response body
	ModelID         string // CodeNoSuchModel: the model id asked for
	ModelType       string // CodeNoSuchModel: the model type it was asked for as
	ProviderID      string // CodeNoSuchProvider: the provider id asked for
	ToolName        string // CodeNoSuchTool / CodeInvalidToolInput: the tool name called
	// AvailableTools is the tool names the call offered (CodeNoSuchTool);
	// nil when the core did not report them.
	AvailableTools []string
	ToolInput      string // CodeInvalidToolInput: the raw argument text the model produced
	// OriginalError is the lookup/parse/validation failure repair could not
	// fix (CodeToolCallRepair), as externally-tagged wire JSON — the same
	// encoding as ToolCall.Error.
	OriginalError json.RawMessage
}

// Error implements the error interface.
func (e *Error) Error() string {
	if e == nil {
		return "aimux: <nil>"
	}
	if e.Message != "" {
		return e.Message
	}
	return fmt.Sprintf("aimux: %s", e.Code.String())
}

// defaultStatus fills the status TokenExpired carries by contract (401) when
// the C ABI reports -1. CodeAPICall statuses are observed, never invented: a
// missing status there means no HTTP response was ever observed (a missing API
// key, an error built without a request, or a transport failure), so it stays
// -1.
func defaultStatus(code Code, status int) int {
	if status == -1 && code == CodeTokenExpired {
		return 401
	}
	return status
}

// RecordingErrorCode mirrors aimux-core recording::RecordingError variants.
// Go keeps compact values 1..6; the C ABI transports them as 100..105. Only WriterGone,
// FlushTimeout and Write are reachable from RecordingTryFlush.
type RecordingErrorCode int

const (
	RecordingErrorInit         RecordingErrorCode = 1
	RecordingErrorOpenFile     RecordingErrorCode = 2
	RecordingErrorSpawn        RecordingErrorCode = 3
	RecordingErrorWriterGone   RecordingErrorCode = 4
	RecordingErrorFlushTimeout RecordingErrorCode = 5
	RecordingErrorWrite        RecordingErrorCode = 6
)

// recordingErrorCodeFromC maps C codes 100..105 to Go values 1..6; false
// for any other value.
func recordingErrorCodeFromC(code int) (RecordingErrorCode, bool) {
	const recordingCodeBase = 99
	if code < recordingCodeBase+int(RecordingErrorInit) || code > recordingCodeBase+int(RecordingErrorWrite) {
		return 0, false
	}
	return RecordingErrorCode(code - recordingCodeBase), true
}

// ffiCodeFromC reports whether code is one of the implementation failures
// detected by the C ABI. Go intentionally maps all of them to a plain error.
func ffiCodeFromC(code int) bool {
	return code >= 200 && code <= 206
}

// String returns the Rust variant name; unmapped codes render as
// "RecordingErrorCode(n)".
func (c RecordingErrorCode) String() string {
	switch c {
	case RecordingErrorInit:
		return "Init"
	case RecordingErrorOpenFile:
		return "OpenFile"
	case RecordingErrorSpawn:
		return "Spawn"
	case RecordingErrorWriterGone:
		return "WriterGone"
	case RecordingErrorFlushTimeout:
		return "FlushTimeout"
	case RecordingErrorWrite:
		return "Write"
	default:
		return fmt.Sprintf("RecordingErrorCode(%d)", int(c))
	}
}

// RecordingError is a recording (RFC-0023) failure, returned by
// RecordingTryFlush. It is independent of *Error — the two share only the
// error interface, as Rust's RecordingError and AiMuxError share only
// std::error::Error.
type RecordingError struct {
	Code    RecordingErrorCode
	Message string
}

// Error implements the error interface.
func (e *RecordingError) Error() string {
	if e == nil {
		return "aimux recording: <nil>"
	}
	if e.Message != "" {
		return e.Message
	}
	return fmt.Sprintf("aimux recording: %s", e.Code.String())
}

// ErrClosed is returned (wrapped, so test with errors.Is) when a Model,
// ProviderHandle, multimodal model or transcription session is used after
// Close. It is the binding's own guard, evaluated before any C call.
var ErrClosed = errors.New("aimux: use of closed handle")

// ErrNotTraced is returned (test with errors.Is) when a trace query runs on a
// model that never went through Trace/TraceAudited. The trace store is keyed
// on the wrapper handle, so a plain model simply has none — the same class of
// misuse as ErrClosed, and guarded in Go before any C call.
var ErrNotTraced = errors.New("aimux: model is not traced; call Trace() first")

// checkUTF8 validates string parameters (param, value pairs) that cross to C.
// A Go string is an arbitrary byte sequence and C.CString passes it through
// verbatim, but the Rust side requires UTF-8 and stops at the first NUL — both
// would come back as a C ABI failure code, or (for the NUL) as a
// silently truncated argument. Cheaper to name the parameter here.
func checkUTF8(pairs ...string) error {
	for i := 0; i+1 < len(pairs); i += 2 {
		v := pairs[i+1]
		if !utf8.ValidString(v) {
			return fmt.Errorf("aimux: %s: must be valid UTF-8", pairs[i])
		}
		if strings.IndexByte(v, 0) >= 0 {
			return fmt.Errorf("aimux: %s: must not contain NUL", pairs[i])
		}
	}
	return nil
}

// checkJSON validates optional raw JSON string parameters (param, value
// pairs) before they cross the C ABI, so malformed opts/config JSON never
// reaches aimux-ffi. "" means "default" and is skipped.
func checkJSON(pairs ...string) error {
	if err := checkUTF8(pairs...); err != nil {
		return err
	}
	for i := 0; i+1 < len(pairs); i += 2 {
		v := pairs[i+1]
		if v == "" {
			continue
		}
		if !json.Valid([]byte(v)) {
			return fmt.Errorf("aimux: %s: invalid JSON", pairs[i])
		}
		if err := checkSurrogates(pairs[i], v); err != nil {
			return err
		}
	}
	return nil
}

// checkSurrogates rejects the \uXXXX escapes serde_json rejects but
// encoding/json accepts: an unpaired surrogate half (json.Valid passes it,
// then serde fails in C with a C ABI failure code). Runs after json.Valid, so
// every backslash here is a well-formed escape inside a string.
func checkSurrogates(param, s string) error {
	for i := 0; i+1 < len(s); {
		if s[i] != '\\' {
			i++
			continue
		}
		if s[i+1] != 'u' {
			i += 2 // \" \\ \/ \b \f \n \r \t — skip the escaped byte
			continue
		}
		hi, ok := hex4(s, i)
		if !ok {
			i += 2
			continue
		}
		i += 6
		switch {
		case hi >= 0xDC00 && hi <= 0xDFFF:
			return fmt.Errorf("aimux: %s: invalid JSON: lone low surrogate \\u%04X", param, hi)
		case hi >= 0xD800 && hi <= 0xDBFF:
			lo, ok := hex4(s, i)
			if !ok || lo < 0xDC00 || lo > 0xDFFF {
				return fmt.Errorf("aimux: %s: invalid JSON: unpaired high surrogate \\u%04X", param, hi)
			}
			i += 6
		}
	}
	return nil
}

// hex4 reads the code point of a \uXXXX escape starting at s[i]; false when
// s[i:] is not such an escape.
func hex4(s string, i int) (int, bool) {
	if i+6 > len(s) || s[i] != '\\' || s[i+1] != 'u' {
		return 0, false
	}
	n, err := strconv.ParseUint(s[i+2:i+6], 16, 32)
	if err != nil {
		return 0, false
	}
	return int(n), true
}

// marshalJSON serializes v for a C string parameter and validates the result.
//
// json.Marshal is NOT self-validating here. It coerces invalid UTF-8 in a Go
// *string* to U+FFFD and escapes NUL, but a json.RawMessage field (or any
// MarshalJSON, e.g. jsonObj) is passed through compact(), which checks JSON
// *syntax* only. So raw non-UTF-8 bytes and lone \uD800–\uDFFF escapes survive
// into the output — the first fails CStr::to_str on the Rust side, the second
// fails serde_json (LoneLeadingSurrogateInHexEscape). Every option struct here
// has such a field (ProviderOptions, BodyOverrides, Audio, Documents, …), so
// the marshalled bytes get the same checkJSON the raw-string entry points get.
func marshalJSON(param string, v any) (string, error) {
	b, err := json.Marshal(v)
	if err != nil {
		return "", fmt.Errorf("aimux: failed to marshal %s: %w", param, err)
	}
	if err := checkJSON(param, string(b)); err != nil {
		return "", err
	}
	return string(b), nil
}

// requireJSON is checkJSON for a mandatory parameter: "" is rejected too.
func requireJSON(param, v string) error {
	if v == "" {
		return fmt.Errorf("aimux: %s: invalid JSON: empty", param)
	}
	return checkJSON(param, v)
}

// checkPromptOpts validates the (prompt_json, opts_json) pair every generate /
// stream entry point takes: the prompt is required, the opts default on "".
func checkPromptOpts(promptJson, optsJson string) error {
	if err := requireJSON("prompt_json", promptJson); err != nil {
		return err
	}
	return checkJSON("opts_json", optsJson)
}
