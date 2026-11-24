package aimux

/*
#cgo CFLAGS: -I${SRCDIR}/../../aimux-ffi/include
#cgo linux LDFLAGS: -L${SRCDIR}/../../target/staticlib-release -L${SRCDIR}/../../target/release -Wl,-Bstatic -laimux_ffi -Wl,-Bdynamic -lpthread -ldl -lm
#cgo darwin LDFLAGS: -L${SRCDIR}/../../target/staticlib-release -L${SRCDIR}/../../target/release -laimux_ffi -framework CoreFoundation
#cgo windows LDFLAGS: -L${SRCDIR}/../../target/staticlib-release -L${SRCDIR}/../../target/release -Wl,-Bstatic -laimux_ffi -Wl,-Bdynamic -lws2_32 -lbcrypt -lntdll -luserenv
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include "aimux.h"

extern void goCallComplete(uintptr_t context, char *result, aimux_error_t *error);
extern void goStreamBatch(uintptr_t context, char **frames, size_t *lengths, size_t count);

static void call_trampoline(char *result, aimux_error_t *error, void *context) {
    goCallComplete((uintptr_t)context, result, error);
}
static aimux_error_t *call_submit(uint64_t handle, const char *op,
    const char *params, uint64_t abort, uintptr_t context) {
    return aimux_call_async(handle, op, params, abort, call_trampoline, (void *)context);
}

// Bound the scan before allocating a Go copy of an untrusted native frame.
static size_t frame_length(const char *frame, size_t limit) {
    if (!frame) return 0;
    size_t n = 0;
    while (n < limit && frame[n]) ++n;
    return n;
}
static void batch_trampoline(const char *const *frames, size_t count, void *context) {
    size_t lengths[64];
    if (count == 0 || count > sizeof(lengths) / sizeof(lengths[0]) || !frames) {
        goStreamBatch((uintptr_t)context, NULL, NULL, 0);
        return;
    }
    for (size_t i = 0; i < count; ++i) {
        // Preserve the Go receiver's data plus control overhead limit.
        lengths[i] = frame_length(frames[i], (1u << 20) + 4096 + 1);
    }
    goStreamBatch((uintptr_t)context, (char **)frames, lengths, count);
}
static aimux_error_t *stream_submit(uint64_t handle, const char *op,
    const char *params, uint64_t abort, uintptr_t context) {
    return aimux_stream_async(handle, op, params, abort, batch_trampoline, (void *)context);
}
// Legacy baseline for the paired benchmark, using the same Go receiver.
static void stream_trampoline(const char *frame, void *context) {
    batch_trampoline(&frame, 1, context);
}
static aimux_error_t *stream_call(uint64_t handle, const char *op,
    const char *params, uint64_t abort, uintptr_t context) {
    return aimux_stream(handle, op, params, abort, stream_trampoline, (void *)context);
}
static void finalize_handle(uint64_t id) {
    aimux_handle_finalizer((void *)(uintptr_t)id);
}
*/
import "C"

import (
	"bytes"
	"encoding/json"
	"fmt"
	"runtime"
	"runtime/cgo"
	"sync"
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
	view := unsafe.Slice((*byte)(unsafe.Pointer(s)), int(C.strlen(s)))
	return bytes.Clone(view)
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
	return json.RawMessage(raw), nil
}

func validateResult(raw json.RawMessage, err error) (json.RawMessage, error) {
	if err != nil {
		return nil, err
	}
	if err := validateJSON("result", raw); err != nil {
		return nil, localError("binding", 205, "ResultSerialization", err.Error())
	}
	return raw, nil
}

func nativeManifest() (json.RawMessage, error) {
	var out *C.char
	e := C.aimux_manifest_json(&out)
	return validateResult(nativeResult(out, e))
}

// Appending NUL also supplies a valid address for empty input. These calls
// borrow only the pointer-free backing array, pinned by cgo until return.
func nativeCallSync(id uint64, op string, params []byte, abort uint64) (json.RawMessage, error) {
	cOp := C.CString(op)
	defer C.free(unsafe.Pointer(cOp))
	params = append(params, 0)
	var out *C.char
	e := C.aimux_call(C.uint64_t(id), cOp, (*C.char)(unsafe.Pointer(&params[0])), C.uint64_t(abort), &out)
	return nativeResult(out, e)
}

type callResult struct {
	json json.RawMessage
	err  error
}

func nativeCall(id uint64, op string, params []byte, abort uint64) (json.RawMessage, error) {
	cOp := C.CString(op)
	params = append(params, 0)
	done := make(chan callResult, 1)
	token := cgo.NewHandle(done)
	defer token.Delete()
	e := C.call_submit(C.uint64_t(id), cOp, (*C.char)(unsafe.Pointer(&params[0])), C.uint64_t(abort), C.uintptr_t(token))
	C.free(unsafe.Pointer(cOp))
	if e != nil {
		return nil, nativeError(e)
	}
	result := <-done
	return result.json, result.err
}

//export goCallComplete
func goCallComplete(token C.uintptr_t, out *C.char, e *C.aimux_error_t) {
	done := cgo.Handle(token).Value().(chan callResult)
	raw, err := nativeResult(out, e)
	done <- callResult{raw, err}
}

type streamCallbacks struct {
	// Native callbacks are serialized, but may run on different OS threads.
	// The Go mutex also makes that ordering visible to the race detector.
	mu     sync.Mutex
	stream *Stream
	done   chan struct{}
	calls  int
}

func nativeStream(id uint64, op string, params []byte, abort uint64, callbacks *streamCallbacks, ready chan<- error) error {
	cOp := C.CString(op)
	params = append(params, 0)
	// Native code keeps only an integer token. Hold it until the terminal
	// callback publishes completion and no later callback can use it.
	callback := cgo.NewHandle(callbacks)
	activeCallbacks.Add(1)
	defer func() { callback.Delete(); activeCallbacks.Add(-1) }()
	e := C.stream_submit(C.uint64_t(id), cOp, (*C.char)(unsafe.Pointer(&params[0])), C.uint64_t(abort), C.uintptr_t(callback))
	C.free(unsafe.Pointer(cOp))
	err := nativeError(e)
	ready <- err
	if err == nil {
		<-callbacks.done
	}
	return err
}

func nativeStreamSync(id uint64, op string, params []byte, abort uint64, callbacks *streamCallbacks) error {
	cOp := C.CString(op)
	defer C.free(unsafe.Pointer(cOp))
	params = append(params, 0)
	token := cgo.NewHandle(callbacks)
	defer token.Delete()
	return nativeError(C.stream_call(C.uint64_t(id), cOp, (*C.char)(unsafe.Pointer(&params[0])), C.uint64_t(abort), C.uintptr_t(token)))
}

//export goStreamBatch
func goStreamBatch(token C.uintptr_t, frames **C.char, lengths *C.size_t, count C.size_t) {
	callbacks := cgo.Handle(token).Value().(*streamCallbacks)
	callbacks.mu.Lock()
	defer func() {
		ended := callbacks.stream.ended
		callbacks.mu.Unlock()
		if ended {
			close(callbacks.done)
		}
	}()
	callbacks.calls++
	s := callbacks.stream
	defer func() {
		if value := recover(); value != nil {
			s.fail(localError("binding", 206, "CallbackFailure", fmt.Sprintf("callback panic: %v", value)))
		}
	}()
	if count == 0 {
		s.fail(localError("binding", 206, "CallbackFailure", "invalid stream batch"))
		return
	}
	sizes := unsafe.Slice(lengths, int(count))
	for i, frame := range unsafe.Slice(frames, int(count)) {
		if frame == nil {
			s.fail(localError("binding", 206, "CallbackFailure", "NULL stream frame"))
			continue
		}
		if sizes[i] > maxFrameBytes {
			s.fail(localError("protocol", 305, "ResourceLimit", "stream frame exceeds the data and control budget"))
			continue
		}
		// RawMessage copies the event and error fields during decode.
		s.receive(unsafe.Slice((*byte)(unsafe.Pointer(frame)), int(sizes[i])))
	}
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
