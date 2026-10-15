package aimux

/*
#cgo CFLAGS: -I${SRCDIR}/../../aimux-ffi/include
#cgo linux LDFLAGS: -L${SRCDIR}/../../target/release -laimux_ffi -lpthread -ldl -lm
#cgo darwin LDFLAGS: -L${SRCDIR}/../../target/release -laimux_ffi
#cgo windows LDFLAGS: -L${SRCDIR}/../../target/release -Wl,-Bstatic -laimux_ffi -Wl,-Bdynamic -lws2_32 -lbcrypt -lntdll -luserenv
#include <stdlib.h>
#include <stdint.h>
#include "aimux.h"

extern void goStreamFrame(uintptr_t context, char *frame);
static void stream_trampoline(const char *frame, void *context) {
    goStreamFrame((uintptr_t)context, (char *)frame);
}
static aimux_error_t *stream_call(uint64_t handle, const char *op,
    const char *params, uint64_t abort, uintptr_t context) {
    return aimux_stream(handle, op, params, abort, stream_trampoline, (void *)context);
}
// Bound the scan before allocating a Go copy of an untrusted native frame.
static size_t frame_length(const char *frame, size_t limit) {
    if (!frame) return 0;
    size_t n = 0;
    while (n < limit && frame[n]) ++n;
    return n;
}
static void finalize_handle(uint64_t id) {
    aimux_handle_finalizer((void *)(uintptr_t)id);
}
*/
import "C"

import (
	"encoding/json"
	"fmt"
	"runtime"
	"runtime/cgo"
	"unsafe"
)

func nativeABIVersion() uint32 { return uint32(C.aimux_abi_version()) }
func nativeDrop(id uint64)     { C.aimux_drop(C.uint64_t(id)) }
func nativeAbort(id uint64)    { C.aimux_abort(C.uint64_t(id)) }
func nativeFinalize(id uint64) { C.finalize_handle(C.uint64_t(id)) }

func ownedString(s *C.char) []byte {
	if s == nil {
		return nil
	}
	defer C.aimux_free_string(s)
	return []byte(C.GoString(s))
}

func nativeError(e *C.aimux_error_t) error {
	if e == nil {
		return nil
	}
	defer C.aimux_error_free(e)
	raw := ownedString(C.aimux_error_json(e))
	result, err := decodeError(raw)
	if err != nil {
		return localError("binding", 205, "ResultSerialization", fmt.Sprintf("invalid native error envelope: %v", err))
	}
	return result
}

func nativeResult(out *C.char, e *C.aimux_error_t) (json.RawMessage, error) {
	raw := ownedString(out)
	if err := nativeError(e); err != nil {
		return nil, err
	}
	if err := validateJSON("result", raw); err != nil {
		return nil, localError("binding", 205, "ResultSerialization", err.Error())
	}
	return json.RawMessage(raw), nil
}

func nativeManifest() (json.RawMessage, error) {
	var out *C.char
	e := C.aimux_manifest_json(&out)
	return nativeResult(out, e)
}

func nativeCall(id uint64, op string, params []byte, abort uint64) (json.RawMessage, error) {
	cOp, cParams := C.CString(op), C.CString(string(params))
	defer C.free(unsafe.Pointer(cOp))
	defer C.free(unsafe.Pointer(cParams))
	var out *C.char
	e := C.aimux_call(C.uint64_t(id), cOp, cParams, C.uint64_t(abort), &out)
	return nativeResult(out, e)
}

func nativeStream(id uint64, op string, params []byte, abort uint64, s *Stream) error {
	cOp, cParams := C.CString(op), C.CString(string(params))
	defer C.free(unsafe.Pointer(cOp))
	defer C.free(unsafe.Pointer(cParams))
	// Only callback state is stored in cgo.Handle. Rust sees an integer token,
	// never a retained Go pointer. Delete only after all callbacks have exited.
	callback := cgo.NewHandle(s)
	lease := newCallbackHandleLease(callback)
	activeCallbacks.Add(1)
	defer func() { lease.Delete(); activeCallbacks.Add(-1) }()
	e := C.stream_call(C.uint64_t(id), cOp, cParams, C.uint64_t(abort), C.uintptr_t(callback))
	return nativeError(e)
}

//export goStreamFrame
func goStreamFrame(token C.uintptr_t, frame *C.char) {
	s := cgo.Handle(token).Value().(*Stream)
	defer func() {
		if value := recover(); value != nil {
			s.fail(localError("binding", 206, "CallbackFailure", fmt.Sprintf("callback panic: %v", value)))
		}
	}()
	if s.failure != nil {
		return
	}
	n := C.frame_length(frame, C.size_t(maxFrameBytes+1))
	if frame == nil {
		s.fail(localError("binding", 206, "CallbackFailure", "NULL stream frame"))
		return
	}
	if n > maxFrameBytes {
		s.fail(localError("protocol", 305, "ResourceLimit", "stream frame exceeds the data and control budget"))
		return
	}
	// Native memory is valid only for this callback. Copy before returning.
	s.receive(C.GoBytes(unsafe.Pointer(frame), C.int(n)))
}

func nativePush(id uint64, data []byte) error {
	var p *C.uint8_t
	if len(data) != 0 {
		p = (*C.uint8_t)(unsafe.Pointer(&data[0]))
	}
	e := C.aimux_operation_push(C.uint64_t(id), p, C.size_t(len(data)))
	runtime.KeepAlive(data)
	return nativeError(e)
}

func nativeNext(id uint64, timeoutMs int64) (PollResult, error) {
	var out *C.char
	var state C.int32_t
	e := C.aimux_operation_next_part(C.uint64_t(id), C.int64_t(timeoutMs), &out, &state)
	raw := ownedString(out)
	if err := nativeError(e); err != nil {
		return PollResult{}, err
	}
	result := PollResult{State: PollState(state)}
	switch result.State {
	case Part:
		if err := validateJSON("operation part", raw); err != nil {
			return PollResult{}, localError("binding", 205, "ResultSerialization", err.Error())
		}
		result.Part = json.RawMessage(raw)
	case Ended, PollTimeout:
		if raw != nil {
			return PollResult{}, localError("binding", 205, "ResultSerialization", "unexpected JSON for poll state")
		}
	default:
		return PollResult{}, localError("binding", 205, "ResultSerialization", "unknown poll state")
	}
	return result, nil
}
