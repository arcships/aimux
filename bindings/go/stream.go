package aimux

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"runtime"
	"strconv"
	"sync/atomic"
	"unicode/utf8"
)

const (
	maxItemBytes = 1 << 20
	// Allow the data item limit plus bounded control-frame overhead.
	maxFrameBytes = maxItemBytes + 4096
	// Reserve one slot for the event a blocked callback is holding.
	// Events retain only their copied payload, separately from control state.
	streamBufferBytes = 8 << 20
	streamBufferSize  = streamBufferBytes/maxItemBytes - 1
)

var activeCallbacks atomic.Int64

// EndFrame is control state, kept outside the data queue so a full queue can
// never discard the outcome. Seq is the canonical decimal-string sequence.
type EndFrame struct {
	Seq     string `json:"seq"`
	Outcome string `json:"outcome"`
	Error   *Error `json:"-"`
}

// Stream has a single native producer and one consumer. Its data channel is
// closed only by that producer, after terminal callback delivery.
// Cancel is safe without a consumer. Always Close a stream abandoned early.
type Stream struct {
	bridge *streamBridge[json.RawMessage]
	emit   func(json.RawMessage) bool
	events chan json.RawMessage
	done   chan struct{}
	cancel context.CancelFunc
	ctx    context.Context
	abort  *Handle
	// Written only by the native producer/callbacks, read after done closes.
	ended    bool
	terminal *EndFrame
	failure  error
	err      error
	seq      uint64
	hasSeq   bool
}

func (h *Handle) Stream(ctx context.Context, op string, params any) (*Stream, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	data, err := encodeParams(op, params)
	if err != nil {
		return nil, err
	}
	id, err := h.ID()
	if err != nil {
		return nil, err
	}
	if err := CheckABI(); err != nil {
		return nil, err
	}
	abort, err := createAbort()
	if err != nil {
		return nil, err
	}
	streamCtx, cancel := context.WithCancel(ctx)
	s := &Stream{ctx: streamCtx, cancel: cancel, abort: abort}
	ready := make(chan error, 1)
	start := make(chan struct{})
	// Start even if the caller cancels now: native admission/terminal delivery
	// and abort cleanup must finish before publishing channel closure.
	s.bridge = newStreamBridge(context.Background(), streamBufferSize, func(_ context.Context, emit func(json.RawMessage) bool) error {
		<-start
		s.emit = emit
		finished := make(chan struct{})
		defer close(finished)
		go func() {
			select {
			case <-streamCtx.Done():
				s.bridge.Cancel()
			case <-finished:
			}
		}()
		s.produce(h, uint64(id), op, data, ready)
		return s.err
	})
	s.events, s.done = s.bridge.parts, s.bridge.done
	close(start)
	if err := <-ready; err != nil {
		<-s.done
		return nil, err
	}
	return s, nil
}

func (s *Stream) produce(owner *Handle, id uint64, op string, params []byte, ready chan<- error) {
	stop := watchAbort(s.ctx, s.abort)
	callbacks := &streamCallbacks{stream: s, done: make(chan struct{})}
	nativeErr := nativeStream(id, op, params, s.abort.id.Load(), callbacks, ready)
	runtime.KeepAlive(owner)
	stop()
	s.abort.Close()
	s.cancel()
	switch {
	case s.failure != nil:
		s.err = s.failure
	case nativeErr != nil:
		s.err = nativeErr
	case s.terminal == nil:
		s.err = localError("binding", 206, "CallbackFailure", "stream returned without an end frame")
	case s.terminal.Error != nil:
		s.err = s.terminal.Error
	case s.terminal.Outcome == "cancelled":
		s.err = localError("aimux", 13, "AbortError", "stream cancelled")
	}
}

func (s *Stream) fail(err error) {
	if s.failure == nil {
		s.failure = err
		s.cancel()
	}
}

func (s *Stream) receive(raw []byte) {
	if !utf8.Valid(raw) {
		s.fail(localError("binding", 206, "CallbackFailure", "stream frame: invalid UTF-8 JSON"))
		return
	}
	var frame struct {
		Type    string          `json:"type"`
		Seq     string          `json:"seq"`
		Event   json.RawMessage `json:"event"`
		Outcome string          `json:"outcome"`
		Error   json.RawMessage `json:"error"`
	}
	decoder := json.NewDecoder(bytes.NewReader(raw))
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&frame); err != nil {
		s.fail(localError("binding", 206, "CallbackFailure", err.Error()))
		return
	}
	if frame.Type == "end" {
		s.ended = true
	} else if s.failure != nil {
		return
	}
	// Decode accepts a stream of values. A callback must contain exactly one.
	if err := decoder.Decode(&struct{}{}); err != io.EOF {
		s.fail(localError("binding", 206, "CallbackFailure", "unexpected data after stream frame"))
		return
	}
	if err := checkSurrogates("stream frame", raw); err != nil {
		s.fail(localError("binding", 206, "CallbackFailure", err.Error()))
		return
	}
	seq, err := strconv.ParseUint(frame.Seq, 10, 64)
	if err != nil || strconv.FormatUint(seq, 10) != frame.Seq || (s.hasSeq && seq <= s.seq) || s.terminal != nil {
		s.fail(localError("binding", 206, "CallbackFailure", "invalid stream sequence or frame after terminal"))
		return
	}
	s.seq, s.hasSeq = seq, true
	switch frame.Type {
	case "event":
		if len(frame.Event) > maxItemBytes {
			s.fail(localError("protocol", 305, "ResourceLimit", "stream event exceeds 1 MiB"))
			return
		}
		if len(frame.Event) == 0 || frame.Outcome != "" || len(frame.Error) != 0 {
			s.fail(localError("binding", 206, "CallbackFailure", "invalid event frame"))
			return
		}
	case "end":
		if len(frame.Event) != 0 {
			s.fail(localError("binding", 206, "CallbackFailure", "unexpected event in terminal"))
			return
		}
		terminal := &EndFrame{Seq: frame.Seq, Outcome: frame.Outcome}
		if len(frame.Error) != 0 {
			var err error
			terminal.Error, err = decodeError(frame.Error)
			if err != nil {
				s.fail(localError("binding", 206, "CallbackFailure", err.Error()))
				return
			}
		}
		switch frame.Outcome {
		case "success":
			if terminal.Error != nil {
				s.fail(localError("binding", 206, "CallbackFailure", "success terminal has an error"))
				return
			}
		case "error":
			if terminal.Error == nil {
				s.fail(localError("binding", 206, "CallbackFailure", "error terminal has no envelope"))
				return
			}
		case "cancelled":
		default:
			s.fail(localError("binding", 206, "CallbackFailure", fmt.Sprintf("invalid terminal outcome %q", frame.Outcome)))
			return
		}
		s.terminal = terminal
	default:
		s.fail(localError("binding", 206, "CallbackFailure", "unknown stream frame type"))
		return
	}
	if frame.Type == "event" {
		s.emit(frame.Event)
	}
}

func (s *Stream) Events() <-chan json.RawMessage { return s.events }
func (s *Stream) Done() <-chan struct{}          { return s.done }
func (s *Stream) Cancel()                        { s.cancel() }
func (s *Stream) Close() error                   { s.Cancel(); <-s.done; return s.Err() }

// Err waits for the producer to close the data channel before reading its
// result. Drain Events first, or Cancel if abandoning consumption.
func (s *Stream) Err() error {
	<-s.done
	return s.err
}

func (s *Stream) Terminal() (*EndFrame, error) {
	<-s.done
	return s.terminal, s.err
}
