package aimux

import (
	"context"
	"errors"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

func bridgeAwait(t *testing.T, done <-chan struct{}) {
	t.Helper()
	select {
	case <-done:
	case <-time.After(5 * time.Second):
		t.Fatal("stream bridge did not make progress")
	}
}

type bridgeMockHandle struct{ deletes atomic.Int32 }

func (h *bridgeMockHandle) Delete() { h.deletes.Add(1) }

func TestStreamBridgeOrderedCompletion(t *testing.T) {
	for _, capacity := range []int{0, 1, 4} {
		handle := &bridgeMockHandle{}
		s := newStreamBridge(context.Background(), capacity, func(ctx context.Context, emit func(int) bool) error {
			defer handle.Delete()
			for i := 0; i < 100; i++ {
				if !emit(i) {
					return ctx.Err()
				}
			}
			return nil
		})
		t.Cleanup(s.Cancel)
		got := 0
		for part := range s.Parts() {
			if part != got {
				t.Fatalf("part = %d, want %d", part, got)
			}
			got++
		}
		if got != 100 || s.Err() != nil {
			t.Fatalf("got %d parts, err = %v", got, s.Err())
		}
		bridgeAwait(t, s.Done())
		if handle.deletes.Load() != 1 {
			t.Fatalf("handle Delete count = %d, want 1", handle.deletes.Load())
		}
	}
}

func TestStreamBridgeErrorPublishedAtChannelClose(t *testing.T) {
	want := errors.New("producer failed")
	for i := 0; i < 1000; i++ {
		s := newStreamBridge(context.Background(), 1, func(context.Context, func(int) bool) error {
			return want
		})
		for range s.Parts() {
			t.Fatal("unexpected part")
		}
		// Do not await Done: Parts closure alone must publish the error.
		if !errors.Is(s.Err(), want) {
			t.Fatalf("Err = %v, want %v", s.Err(), want)
		}
	}
}

func TestStreamBridgeErrBeforeClose(t *testing.T) {
	release := make(chan struct{})
	want := errors.New("terminal")
	s := newStreamBridge(context.Background(), 1, func(context.Context, func(int) bool) error {
		<-release
		return want
	})
	for i := 0; i < 100; i++ {
		if err := s.Err(); err != nil {
			t.Fatalf("early Err = %v", err)
		}
	}
	close(release)
	bridgeAwait(t, s.Done())
	if !errors.Is(s.Err(), want) {
		t.Fatalf("Err = %v, want %v", s.Err(), want)
	}
}

func TestStreamBridgeCanceledBeforeStart(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	var called atomic.Bool
	s := newStreamBridge(ctx, 1, func(context.Context, func(int) bool) error {
		called.Store(true)
		return nil
	})
	bridgeAwait(t, s.Done())
	if called.Load() {
		t.Fatal("producer ran with a canceled context")
	}
	if _, ok := <-s.Parts(); ok {
		t.Fatal("Parts is not closed")
	}
	if !errors.Is(s.Err(), context.Canceled) {
		t.Fatalf("Err = %v", s.Err())
	}
}

func TestStreamBridgeCancelWithoutConsumer(t *testing.T) {
	for _, parentCancellation := range []bool{false, true} {
		ctx, cancel := context.WithCancel(context.Background())
		full := make(chan struct{})
		var sends atomic.Int32
		s := newStreamBridge(ctx, 2, func(_ context.Context, emit func(int) bool) error {
			for i := 0; i < 2; i++ {
				if !emit(i) {
					return errors.New("queue canceled before filling")
				}
				sends.Add(1)
			}
			close(full)
			if emit(2) {
				sends.Add(1)
				return errors.New("bounded queue accepted extra part")
			}
			return nil
		})
		t.Cleanup(cancel)
		t.Cleanup(s.Cancel)
		bridgeAwait(t, full)
		if len(s.parts) != 2 || cap(s.parts) != 2 || sends.Load() != 2 {
			t.Fatal("queue is not bounded at capacity two")
		}
		if parentCancellation {
			cancel()
		} else {
			s.Cancel()
			s.Cancel()
		}
		// No consumer has read a single part. Producer must still terminate.
		bridgeAwait(t, s.Done())
		if !errors.Is(s.Err(), context.Canceled) {
			t.Fatalf("Err = %v", s.Err())
		}
		if sends.Load() != 2 {
			t.Fatal("producer sent past the bounded queue")
		}
		count := 0
		for range s.Parts() {
			count++
		}
		if count != 2 {
			t.Fatalf("buffered count = %d, want 2", count)
		}
	}
}

func TestStreamBridgeCancelKeepsHandleUntilProducerReturns(t *testing.T) {
	handle := &bridgeMockHandle{}
	entered := make(chan struct{})
	canceled := make(chan struct{})
	release := make(chan struct{})
	s := newStreamBridge(context.Background(), 1, func(ctx context.Context, emit func(int) bool) error {
		defer handle.Delete()
		close(entered)
		<-ctx.Done() // Simulates native Abort acknowledgment.
		close(canceled)
		<-release // Simulates a callback still on the native call stack.
		if emit(1) {
			return errors.New("callback emitted after cancellation")
		}
		return nil
	})
	bridgeAwait(t, entered)
	s.Cancel()
	bridgeAwait(t, canceled)
	if handle.deletes.Load() != 0 {
		t.Fatal("callback handle deleted while producer was active")
	}
	select {
	case <-s.Done():
		t.Fatal("consumer canceled and closed the queue before producer returned")
	default:
	}
	if s.Err() != nil {
		t.Fatal("Err published before producer returned")
	}
	close(release)
	bridgeAwait(t, s.Done())
	if handle.deletes.Load() != 1 {
		t.Fatalf("handle Delete count = %d, want 1", handle.deletes.Load())
	}
}

func TestStreamBridgeProducerErrorReleasesHandle(t *testing.T) {
	handle := &bridgeMockHandle{}
	want := errors.New("native startup failed")
	s := newStreamBridge(context.Background(), 1, func(context.Context, func(int) bool) error {
		defer handle.Delete()
		return want
	})
	bridgeAwait(t, s.Done())
	if handle.deletes.Load() != 1 || !errors.Is(s.Err(), want) {
		t.Fatalf("deletes = %d, Err = %v", handle.deletes.Load(), s.Err())
	}
}

func TestStreamBridgeCancellationCompletionRace(t *testing.T) {
	for i := 0; i < 200; i++ {
		ctx, cancel := context.WithCancelCause(context.Background())
		want := errors.New("parent aborted")
		handle := &bridgeMockHandle{}
		started := make(chan struct{})
		release := make(chan struct{})
		s := newStreamBridge(ctx, 1, func(context.Context, func(int) bool) error {
			defer handle.Delete()
			close(started)
			<-release
			return nil
		})
		bridgeAwait(t, started)
		var wg sync.WaitGroup
		wg.Add(2)
		go func() {
			defer wg.Done()
			cancel(want)
		}()
		go func() {
			defer wg.Done()
			close(release)
		}()
		// Polling Err concurrently must not read a producer write unsafely.
		for j := 0; j < 10; j++ {
			_ = s.Err()
		}
		wg.Wait()
		bridgeAwait(t, s.Done())
		err := s.Err()
		if err != nil && !errors.Is(err, want) {
			t.Fatalf("unexpected terminal Err = %v", err)
		}
		s.Cancel() // Cancellation after completion must not change Err.
		if s.Err() != err || handle.deletes.Load() != 1 {
			t.Fatalf("terminal error changed or handle leaked: %v, deletes = %d", s.Err(), handle.deletes.Load())
		}
	}
}
