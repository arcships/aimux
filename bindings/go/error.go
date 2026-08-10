package aimux

import (
	"fmt"
)

// Code is the machine-readable aimux error kind. Values match
// aimux-ffi AimuxErrorCode (append-only; 2..19 = core AiMuxError variants).
type Code int

const (
	CodeOK              Code = 0
	CodeUnknown         Code = 1
	CodeProvider        Code = 2
	CodeHTTP            Code = 3
	CodeJSON            Code = 4
	CodeStream          Code = 5
	CodeTool            Code = 6
	CodeInvalidArgument Code = 7
	CodeInvalidPrompt   Code = 8
	CodeRateLimited     Code = 9
	CodeAuth            Code = 10
	CodeTokenExpired    Code = 11
	CodeModelNotFound   Code = 12
	CodeUnsupported     Code = 13
	CodeNoSuchModel     Code = 14
	CodeUnknownProvider Code = 15
	CodeAPICall         Code = 16
	CodeTimeout         Code = 17
	CodeAborted         Code = 18
	CodeOther           Code = 19
)

// String returns the core error_type name (e.g. "Auth", "RateLimited").
func (c Code) String() string {
	switch c {
	case CodeOK:
		return "OK"
	case CodeUnknown:
		return "Unknown"
	case CodeProvider:
		return "Provider"
	case CodeHTTP:
		return "Http"
	case CodeJSON:
		return "Json"
	case CodeStream:
		return "Stream"
	case CodeTool:
		return "Tool"
	case CodeInvalidArgument:
		return "InvalidArgument"
	case CodeInvalidPrompt:
		return "InvalidPrompt"
	case CodeRateLimited:
		return "RateLimited"
	case CodeAuth:
		return "Auth"
	case CodeTokenExpired:
		return "TokenExpired"
	case CodeModelNotFound:
		return "ModelNotFound"
	case CodeUnsupported:
		return "Unsupported"
	case CodeNoSuchModel:
		return "NoSuchModel"
	case CodeUnknownProvider:
		return "UnknownProvider"
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
//	}
//
// Fields mirror aimux-ffi AimuxError / core helpers:
//   - Code: kind (Auth, RateLimited, …)
//   - Status: HTTP status, or -1
//   - RetryMs: rate-limit hint, or -1 (0 = retry now)
//   - Message: human-readable text
type Error struct {
	Code    Code
	Message string
	Status  int   // HTTP status or -1
	RetryMs int64 // RateLimited hint or -1; 0 = retry immediately
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

// defaultStatus applies the well-known HTTP status defaults that
// Kotlin/Flutter/Node use when the C ABI reports status == -1 (no concrete
// HTTP status): RateLimited→429, Auth/TokenExpired→401, ModelNotFound→404.
// Other kinds keep the raw status (including -1). Mirrors
// bindings/kotlin Errors.kt createByCode and bindings/flutter errors.dart.
func defaultStatus(code Code, status int) int {
	if status != -1 {
		return status
	}
	switch code {
	case CodeRateLimited:
		return 429
	case CodeAuth, CodeTokenExpired:
		return 401
	case CodeModelNotFound:
		return 404
	default:
		return status
	}
}
