package aimux

import (
	"fmt"
)

// Code is the machine-readable aimux error kind. Values match
// aimux-ffi AimuxErrorCode (1..14 = core AiMuxError variants).
//
// Every HTTP-shaped failure arrives as CodeAPICall, classified by Status
// (401 auth, 404 model, 429 rate limit; no status = transport failure).
type Code int

const (
	CodeOK                       Code = 0
	CodeUnknown                  Code = 1
	CodeJSONParse                Code = 2
	CodeInvalidResponseData      Code = 3
	CodeTool                     Code = 4
	CodeInvalidArgument          Code = 5
	CodeInvalidPrompt            Code = 6
	CodeTokenExpired             Code = 7
	CodeUnsupportedFunctionality Code = 8
	CodeNoSuchModel              Code = 9
	CodeNoSuchProvider           Code = 10
	CodeAPICall                  Code = 11
	CodeTimeout                  Code = 12
	CodeAborted                  Code = 13
	CodeOther                    Code = 14
)

// String returns the core error_type name (e.g. "ApiCall", "TokenExpired").
// Unmapped codes render as "Code(n)".
func (c Code) String() string {
	switch c {
	case CodeOK:
		return "OK"
	case CodeUnknown:
		return "Unknown"
	case CodeJSONParse:
		return "JsonParse"
	case CodeInvalidResponseData:
		return "InvalidResponseData"
	case CodeTool:
		return "Tool"
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
	case CodeOther:
		return "Other"
	default:
		return fmt.Sprintf("Code(%d)", int(c))
	}
}

// Error is the structured aimux failure type for Go (openai-go style: one
// struct implementing error, not a class tree).
//
// Transport: returned as the error value of (T, error).
// Inspection:
//
//	var e *aimux.Error
//	if errors.As(err, &e) {
//	    // e.Code, e.Status, e.RetryMs, e.Message
//	    // e.Code == aimux.CodeAPICall && e.Status == 429 → rate limited
//	}
//
// Fields mirror aimux-ffi AimuxError / core helpers:
//   - Code: kind (ApiCall, TokenExpired, …)
//   - Status: HTTP status, or -1 — the classification for CodeAPICall
//     (401 auth, 404 model, 429 rate limit)
//   - RetryMs: rate-limit hint, or -1 (0 = retry now)
//   - Message: human-readable text
type Error struct {
	Code    Code
	Message string
	Status  int   // HTTP status or -1
	RetryMs int64 // retry hint or -1; 0 = retry immediately
	// ErrorValue is the lossless machine-readable source error:
	// externally-tagged aimux-core AiMuxError JSON; empty for failures
	// synthesized at the FFI boundary.
	ErrorValue string
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

// newError builds a local (non-FFI) structured error.
func newError(code Code, message string) *Error {
	return &Error{Code: code, Message: message, Status: -1, RetryMs: -1}
}

// defaultStatus fills the status TokenExpired carries by contract (401) when
// the C ABI reports -1. CodeAPICall statuses are observed, never invented: a
// missing status there means no response arrived (transport failure), so it
// stays -1.
func defaultStatus(code Code, status int) int {
	if status == -1 && code == CodeTokenExpired {
		return 401
	}
	return status
}
