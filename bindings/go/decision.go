package aimux

/*
#cgo CFLAGS: -I${SRCDIR}/../../aimux-ffi
#include <stdlib.h>
#include "aimux-ffi.h"
*/
import "C"

import (
	"context"
	"encoding/json"
	"runtime"
	"unsafe"
)

// DecisionModel calls the official TypeSafe Jev API. Close releases its handle.
// Like other model handles, it must not be copied after first use.
type DecisionModel struct{ h multimodalHandle }

// NewJevDecision uses the official TypeSafe endpoint.
func NewJevDecision(apiKey, modelID string) (*DecisionModel, error) {
	return NewJevDecisionWithEndpoint(apiKey, modelID, "", "")
}

// NewJevDecisionWithEndpoint accepts a complete POST URL and optional provenance.
// Empty endpoint/source selects the official endpoint and native probabilities.
func NewJevDecisionWithEndpoint(apiKey, modelID, endpoint, probabilitySource string) (*DecisionModel, error) {
	if err := checkUTF8("api_key", apiKey, "model_id", modelID, "endpoint", endpoint,
		"probability_source", probabilitySource); err != nil {
		return nil, err
	}
	key, id, cleanup := cstringPair(apiKey, modelID)
	defer cleanup()
	var url, source *C.char
	if endpoint != "" {
		url = C.CString(endpoint)
		defer C.free(unsafe.Pointer(url))
	}
	if probabilitySource != "" {
		source = C.CString(probabilitySource)
		defer C.free(unsafe.Pointer(source))
	}
	var handle C.uint64_t
	if err := expectAimuxError(C.aimux_jev_decision_new_with_probability_source(key, id, url, source, &handle)); err != nil {
		return nil, err
	}
	model := &DecisionModel{}
	model.h.handle.Store(uint64(handle))
	runtime.SetFinalizer(model, func(m *DecisionModel) { m.Close() })
	return model, nil
}

func (m *DecisionModel) Close() error { closeMultimodal(m, &m.h); return nil }

// Capabilities returns DecisionCapabilities JSON, without a network request.
func (m *DecisionModel) Capabilities() (string, error) {
	return callFFIString(m, &m.h, func(handle C.uint64_t, out **C.char) *C.aimux_error_t {
		return C.aimux_decision_capabilities(handle, out)
	})
}

// Decide accepts a value serializable as DecisionCallOptions, preserving native
// structured instructions/criteria. It returns canonical DecisionResult JSON.
func (m *DecisionModel) Decide(options any) (string, error) {
	return m.DecideContext(context.Background(), options)
}

func (m *DecisionModel) DecideContext(ctx context.Context, options any) (string, error) {
	payload, err := json.Marshal(options)
	if err != nil {
		return "", err
	}
	return m.DecideJSONContext(ctx, string(payload))
}

// DecideJSONContext is the raw JSON boundary with context cancellation.
func (m *DecisionModel) DecideJSONContext(ctx context.Context, optionsJSON string) (string, error) {
	if err := checkUTF8("options", optionsJSON); err != nil {
		return "", err
	}
	if err := ctx.Err(); err != nil {
		return "", err
	}
	ptr := C.CString(optionsJSON)
	defer C.free(unsafe.Pointer(ptr))
	abort := C.aimux_abort_signal_new()
	done := make(chan struct{})
	stop := context.AfterFunc(ctx, func() {
		C.aimux_abort_signal_abort(abort)
		close(done)
	})
	defer func() {
		if !stop() {
			<-done
		}
		C.aimux_abort_signal_drop(abort)
	}()
	result, err := callFFIString(m, &m.h, func(handle C.uint64_t, out **C.char) *C.aimux_error_t {
		return C.aimux_decide_with_abort(handle, ptr, abort, out)
	})
	if err != nil && ctx.Err() != nil {
		return "", ctx.Err()
	}
	return result, err
}
