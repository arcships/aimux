// Package aimux binds the RFC-0039 C ABI. Operation names and wire DTOs are
// defined by the runtime manifest; the binding does not add a retry layer.
package aimux

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"runtime"
	"sync"
	"sync/atomic"

	"github.com/arcships/aimux/bindings/go/generated"
)

const ABIVersion uint32 = generated.ABI_VERSION

var abiCheck struct {
	sync.Once
	err error
}

// Handle owns one native registry entry. Do not copy it. Close is idempotent
// and safe alongside calls: native lookup either clones the entry or reports
// InvalidHandle. Close does not cancel calls that already cloned a model.
type Handle struct{ id atomic.Uint64 }

// AdoptHandle takes ownership of a nonzero handle returned by this C runtime.
// Never adopt the same ID twice or a handle from a different owner/runtime.
func AdoptHandle(id HandleID) (*Handle, error) {
	if id == 0 {
		return nil, localError("binding", 203, "InvalidHandle", "cannot own global handle 0")
	}
	h := &Handle{}
	h.id.Store(uint64(id))
	runtime.SetFinalizer(h, finalizeHandle)
	return h, nil
}

func finalizeHandle(h *Handle) {
	if id := h.id.Swap(0); id != 0 {
		nativeFinalize(id)
	}
}

func (h *Handle) Close() error {
	if h != nil {
		if id := h.id.Swap(0); id != 0 {
			runtime.SetFinalizer(h, nil)
			nativeDrop(id)
		}
		runtime.KeepAlive(h)
	}
	return nil
}

func (h *Handle) ID() (HandleID, error) {
	if h != nil {
		if id := h.id.Load(); id != 0 {
			return HandleID(id), nil
		}
	}
	return 0, ErrClosed
}

// CheckABI checks the ABI major and exact generated operation version once,
// before any operation is executed.
func CheckABI() error {
	abiCheck.Do(func() {
		actual := nativeABIVersion()
		if actual>>16 != ABIVersion>>16 {
			abiCheck.err = localError("protocol", 302, "UnsupportedOpsVersion", fmt.Sprintf("incompatible ABI version %#x", actual))
			return
		}
		raw, err := nativeManifest()
		if err != nil {
			abiCheck.err = err
			return
		}
		var manifest struct {
			ABI uint32 `json:"abiVersion"`
			Ops string `json:"opsVersion"`
		}
		if err := json.Unmarshal(raw, &manifest); err != nil {
			abiCheck.err = localError("binding", 205, "ResultSerialization", err.Error())
		} else if manifest.ABI != actual || manifest.Ops != generated.OPS_VERSION {
			abiCheck.err = localError("protocol", 302, "UnsupportedOpsVersion", "runtime manifest does not match the binding operation version")
		}
	})
	return abiCheck.err
}

func Manifest() (json.RawMessage, error) {
	if err := CheckABI(); err != nil {
		return nil, err
	}
	return nativeManifest()
}

// Call invokes a global operation. Pass an explicit object, e.g. struct{}{},
// for no parameters. Nil encodes JSON null and is never normalized to {}.
func Call(ctx context.Context, op string, params any) (json.RawMessage, error) {
	return call(ctx, nil, op, params)
}

func (h *Handle) Call(ctx context.Context, op string, params any) (json.RawMessage, error) {
	if h == nil {
		return nil, ErrClosed
	}
	return call(ctx, h, op, params)
}

func call(ctx context.Context, h *Handle, op string, params any) (json.RawMessage, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	data, err := encodeParams(op, params)
	if err != nil {
		return nil, err
	}
	var id HandleID
	if h != nil {
		id, err = h.ID()
		if err != nil {
			return nil, err
		}
	}
	defer runtime.KeepAlive(h)
	if err := CheckABI(); err != nil {
		return nil, err
	}
	abort, err := createAbort()
	if err != nil {
		return nil, err
	}
	defer abort.Close()
	stop := watchAbort(ctx, abort)
	defer stop()
	return nativeCall(uint64(id), op, data, abort.id.Load())
}

// CallInto decodes any canonical result DTO. A nil handle selects global scope.
// Generated DTOs preserve field presence and opaque JSON; use Call to retain unknown fields.
func CallInto[T any](ctx context.Context, h *Handle, op string, params any) (*T, error) {
	raw, err := call(ctx, h, op, params)
	if err != nil {
		return nil, err
	}
	var result T
	if err := json.Unmarshal(raw, &result); err != nil {
		return nil, localError("binding", 205, "ResultSerialization", err.Error())
	}
	return &result, nil
}

// Create invokes a handle-producing operation in global scope (parent nil)
// or on an existing parent. The new entry owns its native dependencies.
func Create(ctx context.Context, parent *Handle, op string, params any) (*Handle, error) {
	raw, err := call(ctx, parent, op, params)
	if err != nil {
		return nil, err
	}
	return handleFromResult(raw)
}

func handleFromResult(raw []byte) (*Handle, error) {
	// Both shapes are current operation results: for example provider.model
	// returns an object, while transcription.start returns a decimal string.
	raw = bytes.TrimSpace(raw)
	// Constructor objects share the generated handle-only result shape.
	var result RecorderHandleResult
	var err error
	if len(raw) > 0 && raw[0] == '"' {
		err = json.Unmarshal(raw, &result.Handle)
	} else {
		err = json.Unmarshal(raw, &result)
	}
	if err != nil {
		return nil, localError("binding", 205, "ResultSerialization", err.Error())
	}
	if result.Handle == 0 {
		return nil, localError("binding", 205, "ResultSerialization", "constructor returned no owned handle")
	}
	return AdoptHandle(result.Handle)
}

func createAbort() (*Handle, error) {
	raw, err := nativeCall(0, "abort.create", []byte(`{}`), 0)
	if err != nil {
		return nil, err
	}
	return handleFromResult(raw)
}

// The stop function joins the watcher before the abort handle can be dropped.
func watchAbort(ctx context.Context, abort *Handle) func() {
	stop, done := make(chan struct{}), make(chan struct{})
	go func() {
		defer close(done)
		select {
		case <-ctx.Done():
			signalAbort(abort)
		case <-stop:
		}
	}()
	return func() { close(stop); <-done }
}

func signalAbort(abort *Handle) {
	if abort != nil {
		nativeAbort(abort.id.Load())
		runtime.KeepAlive(abort)
	}
}
