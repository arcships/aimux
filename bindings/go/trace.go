// trace.go — RFC-0015 cache probing (C ABI path).
//
// Wraps aimux_trace_new / aimux_trace_new_audited / aimux_trace_aggregate /
// aimux_trace_session_chain / aimux_trace_export_jsonl / aimux_trace_clear.

package aimux

/*
#include <stdlib.h>
#include "aimux-ffi.h"
*/
import "C"

import (
	"errors"
	"fmt"
	"unsafe"
)

// Trace wraps this model in a cache-probe layer (RFC-0015). The returned
// model records fingerprints/verdicts on every GenerateText/StreamText call
// and exposes the trace query API. Caller must Close the returned model.
func (m *Model) Trace() (*Model, error) {
	return m.traceWrap(false, 0)
}

// TraceAudited wraps this model in a probe layer with the built-in rules
// auditor. strict = strict mode (self-hosted single instance); false =
// shared mode (safe default).
func (m *Model) TraceAudited(strict bool) (*Model, error) {
	strictInt := 0
	if strict {
		strictInt = 1
	}
	return m.traceWrap(true, C.int(strictInt))
}

func (m *Model) traceWrap(audited bool, strict C.int) (*Model, error) {
	handle, release, err := m.acquireHandle()
	if err != nil {
		return nil, err
	}
	defer release()

	var ptr *C.char
	if audited {
		ptr = C.aimux_trace_new_audited(C.uint64_t(handle), strict)
	} else {
		ptr = C.aimux_trace_new(C.uint64_t(handle))
	}
	if ptr == nil {
		return nil, errors.New("aimux: trace_new returned null")
	}
	// parseHandleJSON frees the C string (defer inside it) — do NOT free
	// here too (double free).

	handle, err = parseHandleJSON(ptr)
	if err != nil {
		return nil, err
	}
	return &Model{handle: handle}, nil
}

// TraceAggregate returns aggregated probe statistics (JSON TraceStats[]),
// filtered by an optional serialized TraceFilter ("" = all).
func (m *Model) TraceAggregate(filterJson string) (string, error) {
	return m.traceQuery(filterJson, func(h C.uint64_t, c *C.char) *C.char {
		return C.aimux_trace_aggregate(h, c)
	})
}

// TraceSessionChain returns one session's chain view (JSON SessionChainView).
func (m *Model) TraceSessionChain(sessionId string) (string, error) {
	return m.traceQuery(sessionId, func(h C.uint64_t, c *C.char) *C.char {
		return C.aimux_trace_session_chain(h, c)
	})
}

// TraceExportJsonl returns all probe records as JSONL (one TraceRecord per
// line).
func (m *Model) TraceExportJsonl() (string, error) {
	handle, release, err := m.acquireHandle()
	if err != nil {
		return "", err
	}
	defer release()

	ptr := C.aimux_trace_export_jsonl(C.uint64_t(handle))
	if ptr == nil {
		return "", errors.New("aimux: trace_export_jsonl returned null")
	}
	defer C.aimux_free_string(ptr)

	result := C.GoString(ptr)
	if msg := extractError(result); msg != "" {
		return "", fmt.Errorf("aimux: %s", msg)
	}
	return result, nil
}

// TraceClear drops all probe records of this model.
func (m *Model) TraceClear() error {
	handle, release, err := m.acquireHandle()
	if err != nil {
		return err
	}
	defer release()

	if rc := C.aimux_trace_clear(C.uint64_t(handle)); rc != 0 {
		return errors.New("aimux: trace_clear failed (invalid handle)")
	}
	return nil
}

// traceQuery runs a query taking one C string argument.
func (m *Model) traceQuery(arg string, call func(C.uint64_t, *C.char) *C.char) (string, error) {
	handle, release, err := m.acquireHandle()
	if err != nil {
		return "", err
	}
	defer release()

	cArg := C.CString(arg)
	defer C.free(unsafe.Pointer(cArg))

	ptr := call(C.uint64_t(handle), cArg)
	if ptr == nil {
		return "", errors.New("aimux: trace query returned null")
	}
	defer C.aimux_free_string(ptr)

	result := C.GoString(ptr)
	if msg := extractError(result); msg != "" {
		return "", fmt.Errorf("aimux: %s", msg)
	}
	return result, nil
}
