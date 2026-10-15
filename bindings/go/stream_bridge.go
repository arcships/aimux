package aimux

import (
	"context"
	"sync"
)

// streamBridge runs one producer with a bounded queue. The producer must return
// only after all callbacks have stopped, and must honor ctx when doing native
// work (for example, by mapping cancellation to the native Abort operation).
// A callback must stop emitting when emit returns false and must not retain emit
// after the producer returns.
type streamBridge[T any] struct {
	parts  chan T
	done   chan struct{}
	cancel context.CancelFunc
	mu     sync.Mutex
	err    error // Read only after done closes, under mu.
}

// newStreamBridge starts the producer. A zero capacity uses an unbuffered queue;
// a negative capacity is a programming error, as with make(chan T, capacity).
func newStreamBridge[T any](ctx context.Context, capacity int, produce func(context.Context, func(T) bool) error) *streamBridge[T] {
	if ctx == nil {
		ctx = context.Background()
	}
	ctx, cancel := context.WithCancel(ctx)
	s := &streamBridge[T]{
		parts:  make(chan T, capacity),
		done:   make(chan struct{}),
		cancel: cancel,
	}
	go s.run(ctx, produce)
	return s
}

func (s *streamBridge[T]) run(ctx context.Context, produce func(context.Context, func(T) bool) error) {
	defer s.cancel()
	var err error
	if ctx.Err() == nil {
		err = produce(ctx, func(part T) bool {
			// Prefer cancellation already observed before this callback, even if
			// the queue happens to have room. The select also releases a callback
			// blocked on a full queue when nobody is consuming.
			if ctx.Err() != nil {
				return false
			}
			select {
			case s.parts <- part:
				return true
			case <-ctx.Done():
				return false
			}
		})
	}
	if cause := context.Cause(ctx); cause != nil {
		err = cause
	}
	// Only this producer goroutine closes the queue. Cancellation never does.
	s.mu.Lock()
	s.err = err
	close(s.parts)
	close(s.done)
	s.mu.Unlock()
}

func (s *streamBridge[T]) Parts() <-chan T { return s.parts }

// Done closes after the producer and its callbacks have returned and Parts has
// closed. It can be awaited without draining Parts after cancellation.
func (s *streamBridge[T]) Done() <-chan struct{} { return s.done }

// Err returns the terminal error after Parts closes. Before then it returns nil,
// without reading the producer's error. Consumers should drain Parts and await
// its closure before inspecting Err.
func (s *streamBridge[T]) Err() error {
	// Serialize the two channel closes so a consumer observing Parts close
	// cannot miss the terminal error in the interval before Done closes.
	s.mu.Lock()
	defer s.mu.Unlock()
	select {
	case <-s.done:
		return s.err
	default:
		return nil
	}
}

// Cancel is concurrent-safe and never waits for a consumer or the producer.
func (s *streamBridge[T]) Cancel() { s.cancel() }

// callbackHandle is implemented by runtime/cgo.Handle without importing cgo
// into the bridge. Tests can substitute a pure Go handle.
type callbackHandle interface {
	Delete()
}

// callbackHandleLease owns one cgo.Handle. Keep and share the lease by pointer;
// do not copy it or delete the original handle separately. The producer should
// defer lease.Delete immediately after allocating its callback handle, so every
// return path releases it after the synchronous native call and callbacks end.
// Cancel must not delete the handle while callbacks can still access it.
type callbackHandleLease struct {
	once   sync.Once
	handle callbackHandle
}

func newCallbackHandleLease(handle callbackHandle) *callbackHandleLease {
	return &callbackHandleLease{handle: handle}
}

func (h *callbackHandleLease) Delete() {
	if h == nil {
		return
	}
	h.once.Do(func() {
		if h.handle != nil {
			h.handle.Delete()
			h.handle = nil
		}
	})
}
