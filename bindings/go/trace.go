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
	"runtime"
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
	handle, err := m.handle()
	if err != nil {
		return nil, err
	}
	defer runtime.KeepAlive(m)

	var cerr C.AimuxError
	C.aimux_error_clear(&cerr)
	var h C.uint64_t
	if audited {
		h = C.aimux_trace_new_audited(C.uint64_t(handle), strict, &cerr)
	} else {
		h = C.aimux_trace_new(C.uint64_t(handle), &cerr)
	}
	return wrapHandleU64(h, &cerr)
}

// TraceAggregate returns aggregated probe statistics (JSON TraceStats[]),
// filtered by an optional serialized TraceFilter ("" = all).
func (m *Model) TraceAggregate(filterJson string) (string, error) {
	return m.traceQuery(filterJson, func(h C.uint64_t, c *C.char, err *C.AimuxError) *C.char {
		return C.aimux_trace_aggregate(h, c, err)
	})
}

// TraceSessionChain returns one session's chain view (JSON SessionChainView).
func (m *Model) TraceSessionChain(sessionId string) (string, error) {
	return m.traceQuery(sessionId, func(h C.uint64_t, c *C.char, err *C.AimuxError) *C.char {
		return C.aimux_trace_session_chain(h, c, err)
	})
}

// TraceExportJsonl returns all probe records as JSONL (one TraceRecord per
// line).
func (m *Model) TraceExportJsonl() (string, error) {
	handle, err := m.handle()
	if err != nil {
		return "", err
	}
	defer runtime.KeepAlive(m)

	return ffiString(func(cerr *C.AimuxError) *C.char {
		return C.aimux_trace_export_jsonl(C.uint64_t(handle), cerr)
	})
}

// TraceClear drops all probe records of this model.
func (m *Model) TraceClear() error {
	handle, err := m.handle()
	if err != nil {
		return err
	}
	defer runtime.KeepAlive(m)

	if rc := C.aimux_trace_clear(C.uint64_t(handle)); rc != 0 {
		return newError(CodeInvalidArgument, "aimux: trace_clear failed (invalid handle)")
	}
	return nil
}

// traceQuery runs a query taking one C string argument.
func (m *Model) traceQuery(arg string, call func(C.uint64_t, *C.char, *C.AimuxError) *C.char) (string, error) {
	handle, err := m.handle()
	if err != nil {
		return "", err
	}
	defer runtime.KeepAlive(m)

	cArg := C.CString(arg)
	defer C.free(unsafe.Pointer(cArg))

	return ffiString(func(cerr *C.AimuxError) *C.char {
		return call(C.uint64_t(handle), cArg, cerr)
	})
}
