package aimux

import (
	"context"
	"encoding/json"
	"runtime"
	"sync"
	"time"
)

type PollState int32

const (
	Part        PollState = 1
	Ended       PollState = 2
	PollTimeout PollState = 3
)

type PollResult struct {
	State PollState
	Part  json.RawMessage
}

// Operation owns its native operation and a cancellation signal for its
// entire lifetime, including waits for push or poll queue capacity.
type Operation struct {
	handle   *Handle
	abort    *Handle
	stop     func()
	stopOnce sync.Once
}

// StartOperation calls transcription.start. The context remains connected
// to the native Abort until the operation ends or Close returns.
func (h *Handle) StartOperation(ctx context.Context, params any) (*Operation, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	data, err := encodeParams("transcription.start", params)
	if err != nil {
		return nil, err
	}
	id, err := h.ID()
	if err != nil {
		return nil, err
	}
	defer runtime.KeepAlive(h)
	if err := CheckABI(); err != nil {
		return nil, err
	}
	abort, err := createAbort()
	if err != nil {
		return nil, err
	}
	stop := watchAbort(ctx, abort)
	raw, err := nativeCall(uint64(id), "transcription.start", data, abort.id.Load())
	if err != nil {
		stop()
		abort.Close()
		return nil, err
	}
	handle, err := handleFromResult(raw)
	if err != nil {
		stop()
		abort.Close()
		return nil, err
	}
	o := &Operation{handle: handle, abort: abort, stop: stop}
	runtime.SetFinalizer(o, func(o *Operation) { go o.Close() })
	return o, nil
}

func (o *Operation) ID() (HandleID, error) {
	if o == nil {
		return 0, ErrClosed
	}
	return o.handle.ID()
}

func (o *Operation) Push(data []byte) error {
	id, err := o.ID()
	if err != nil {
		return err
	}
	defer runtime.KeepAlive(o)
	if len(data) > maxItemBytes {
		return localError("protocol", 305, "ResourceLimit", "audio chunk exceeds 1 MiB")
	}
	return nativePush(uint64(id), data)
}

// NextPart uses PART=1, ENDED=2 and TIMEOUT=3. Timeout must be nonnegative;
// zero polls without waiting. Only one reader is allowed by
// the native operation. An operation error is delivered once, then ENDED.
func (o *Operation) NextPart(timeout time.Duration) (PollResult, error) {
	if timeout < 0 {
		return PollResult{}, localError("protocol", 301, "InvalidParams", "timeout must be nonnegative")
	}
	id, err := o.ID()
	if err != nil {
		return PollResult{}, err
	}
	defer runtime.KeepAlive(o)
	ms := timeout.Milliseconds()
	if timeout > 0 && ms == 0 {
		ms = 1
	}
	result, err := nativeNext(uint64(id), ms)
	if err == nil && result.State == Ended {
		o.stopOnce.Do(o.stop)
	}
	return result, err
}

func (o *Operation) InputDone(ctx context.Context) error {
	if o == nil {
		return ErrClosed
	}
	_, err := o.handle.Call(ctx, "operation.inputDone", struct{}{})
	runtime.KeepAlive(o)
	return err
}

func (o *Operation) Status(ctx context.Context) (json.RawMessage, error) {
	if o == nil {
		return nil, ErrClosed
	}
	result, err := o.handle.Call(ctx, "operation.status", struct{}{})
	runtime.KeepAlive(o)
	return result, err
}

func (o *Operation) Cancel() {
	if o != nil {
		signalAbort(o.abort)
		runtime.KeepAlive(o)
	}
}

func (o *Operation) Close() error {
	if o == nil {
		return nil
	}
	// Stop/join the context watcher before releasing its signal handle.
	o.Cancel()
	o.stopOnce.Do(o.stop)
	o.handle.Close()
	o.abort.Close()
	runtime.KeepAlive(o)
	return nil
}
