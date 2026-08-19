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

	// [C ABI]: only a dead handle fails, and the closed guard above already
	// excludes that — but a C ABI failure is returned, never panicked.
	var h C.uint64_t
	var e *C.aimux_error_t
	if audited {
		e = C.aimux_trace_new_audited(C.uint64_t(handle), strict, &h)
	} else {
		e = C.aimux_trace_new(C.uint64_t(handle), &h)
	}
	if err := expectFfiError(e); err != nil {
		return nil, err
	}
	t := &Model{traced: true}
	t.id.Store(uint64(h))
	runtime.SetFinalizer(t, func(t *Model) { t.Close() })
	return t, nil
}

// TraceAggregate returns aggregated probe statistics (JSON TraceStats[]),
// filtered by an optional serialized TraceFilter ("" = all).
// Returns ErrNotTraced unless the receiver came from Trace/TraceAudited.
func (m *Model) TraceAggregate(filterJson string) (string, error) {
	// State guard first: an untraced model reports ErrNotTraced whatever the
	// filter looks like.
	if err := m.checkTraced(); err != nil {
		return "", err
	}
	if err := checkJSON("filter_json", filterJson); err != nil {
		return "", err
	}
	if filterJson == "" {
		filterJson = "{}" // C requires JSON; keep the "" = all promise locally
	}
	return m.traceQuery(filterJson, func(h C.uint64_t, c *C.char, out **C.char) *C.aimux_error_t {
		return C.aimux_trace_aggregate(h, c, out)
	})
}

// TraceSessionChain returns one session's chain view (JSON SessionChainView).
// Returns ErrNotTraced unless the receiver came from Trace/TraceAudited.
func (m *Model) TraceSessionChain(sessionId string) (string, error) {
	if err := m.checkTraced(); err != nil {
		return "", err
	}
	if err := checkUTF8("session_id", sessionId); err != nil {
		return "", err
	}
	return m.traceQuery(sessionId, func(h C.uint64_t, c *C.char, out **C.char) *C.aimux_error_t {
		return C.aimux_trace_session_chain(h, c, out)
	})
}

// TraceExportJsonl returns all probe records as JSONL (one TraceRecord per
// line). Returns ErrNotTraced unless the receiver came from
// Trace/TraceAudited.
func (m *Model) TraceExportJsonl() (string, error) {
	handle, err := m.acquireTraced()
	if err != nil {
		return "", err
	}
	defer runtime.KeepAlive(m)

	// [C ABI] per aimux-ffi.h: this call carries no AiMuxError code.
	return ffiStringWithFfiError(func(out **C.char) *C.aimux_error_t {
		return C.aimux_trace_export_jsonl(C.uint64_t(handle), out)
	})
}

// TraceClear drops all probe records of this model.
// Returns ErrNotTraced unless the receiver came from Trace/TraceAudited.
func (m *Model) TraceClear() error {
	handle, err := m.acquireTraced()
	if err != nil {
		return err
	}
	defer runtime.KeepAlive(m)

	// [C ABI]: fails only for a dead handle, which the closed guard excludes.
	return expectFfiError(C.aimux_trace_clear(C.uint64_t(handle)))
}

// checkTraced is the state guard every trace query needs: the trace store is
// keyed on the wrapper handle, so on a plain model the C call reports a
// missing handle — really user misuse, and a much better message here. traced
// is set at construction and never mutated, so it reads without the lock.
func (m *Model) checkTraced() error {
	if m == nil || !m.traced {
		return ErrNotTraced
	}
	return nil
}

// acquireTraced is handle plus that guard, covering all five query
// methods at once.
func (m *Model) acquireTraced() (uint64, error) {
	if err := m.checkTraced(); err != nil {
		return 0, err
	}
	return m.handle()
}

// traceQuery runs a query taking one C string argument.
func (m *Model) traceQuery(arg string, call func(C.uint64_t, *C.char, **C.char) *C.aimux_error_t) (string, error) {
	handle, err := m.acquireTraced()
	if err != nil {
		return "", err
	}
	defer runtime.KeepAlive(m)

	cArg := C.CString(arg)
	defer C.free(unsafe.Pointer(cArg))

	return ffiString(func(out **C.char) *C.aimux_error_t {
		return call(C.uint64_t(handle), cArg, out)
	})
}
