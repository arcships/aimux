package aimux

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"runtime"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/arcships/aimux/bindings/go/generated"
)

func fixture(t *testing.T) {
	t.Helper()
	if os.Getenv("AIMUX_TEST_ABI") != "1" {
		t.Skip("deterministic C fixture only; run ./test-abi.sh")
	}
}

func fixtureHandle(t *testing.T) *Handle {
	t.Helper()
	fixture(t)
	h, err := Create(context.Background(), nil, "test.create", struct{}{})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { h.Close() })
	return h
}

func await(t *testing.T, done <-chan struct{}) {
	t.Helper()
	select {
	case <-done:
	case <-time.After(3 * time.Second):
		t.Fatal("native producer failed to stop")
	}
}

func awaitNativeWait(t *testing.T, field string) {
	t.Helper()
	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) {
		stats, err := CallInto[map[string]int](context.Background(), nil, "test.stats", struct{}{})
		if err != nil {
			t.Fatal(err)
		}
		if (*stats)[field] != 0 {
			return
		}
		runtime.Gosched()
	}
	t.Fatalf("native did not enter %s", field)
}

func checkCode(t *testing.T, err error, kind string, code int32) *Error {
	t.Helper()
	var native *Error
	if !errors.As(err, &native) {
		t.Fatalf("expected structured error, got %T: %v", err, err)
	}
	if native.Envelope.Kind != ErrorDomain(kind) || native.Envelope.Code != code {
		t.Fatalf("unexpected envelope: %+v", native.Envelope)
	}
	return native
}

func TestManifestRequiresExactGeneratedVersionAndDigest(t *testing.T) {
	for _, tc := range []struct {
		ops, digest string
		valid       bool
	}{
		{generated.OPS_VERSION, generated.MANIFEST_DIGEST, true},
		{"1.1", generated.MANIFEST_DIGEST, false},
		{generated.OPS_VERSION, "wrong-digest", false},
		{generated.OPS_VERSION, "", false},
	} {
		raw, err := json.Marshal(map[string]any{"abiVersion": ABIVersion, "opsVersion": tc.ops, "digest": tc.digest})
		if err != nil {
			t.Fatal(err)
		}
		err = validateManifest(ABIVersion, raw)
		if tc.valid {
			if err != nil {
				t.Fatal(err)
			}
			continue
		}
		checkCode(t, err, "protocol", 302)
		for _, value := range []string{tc.ops, generated.OPS_VERSION, tc.digest, generated.MANIFEST_DIGEST} {
			if !strings.Contains(err.Error(), value) {
				t.Fatalf("mismatch omits %q: %v", value, err)
			}
		}
	}
}

func TestRuntimeManifest(t *testing.T) {
	raw, err := Manifest()
	if err != nil {
		t.Fatal(err)
	}
	var manifest struct {
		ABI uint32 `json:"abiVersion"`
		Ops string `json:"opsVersion"`
	}
	if err := json.Unmarshal(raw, &manifest); err != nil {
		t.Fatal(err)
	}
	if manifest.ABI>>16 != generated.ABI_VERSION>>16 || manifest.Ops != generated.OPS_VERSION {
		t.Fatalf("unexpected manifest: %s", raw)
	}
}

func TestNativeErrorEnvelopeDomainsAndOwnership(t *testing.T) {
	fixture(t)
	for _, c := range []struct {
		op, kind string
		code     int32
	}{
		{"test.error", "aimux", 11}, {"test.recording", "recording", 100},
		{"test.binding", "binding", 204}, {"test.protocol", "protocol", 303},
	} {
		_, err := Call(context.Background(), c.op, struct{}{})
		native := checkCode(t, fmt.Errorf("wrapped: %w", err), c.kind, c.code)
		if native.Envelope.RetryAfterMs == nil || *native.Envelope.RetryAfterMs != 0 {
			t.Fatal("lost explicit zero retry hint")
		}
		var payload map[string]json.RawMessage
		if err := json.Unmarshal(native.Envelope.Error, &payload); err != nil {
			t.Fatal(err)
		}
		if string(payload["data"]) != "null" || string(payload["nested"]) != `{"kept":7}` {
			t.Fatalf("lost payload: %s", native.Envelope.Error)
		}
		var all map[string]json.RawMessage
		if err := json.Unmarshal(native.RawEnvelope, &all); err != nil {
			t.Fatal(err)
		}
		if string(all["future"]) != "true" {
			t.Fatal("unknown envelope field lost")
		}
	}
	stats, err := CallInto[struct{ Strings, Errors int }](context.Background(), nil, "test.stats", struct{}{})
	if err != nil {
		t.Fatal(err)
	}
	if stats.Strings != 0 || stats.Errors != 0 {
		t.Fatalf("native owned pointers leaked: %+v", stats)
	}
}

func TestNativeParamsKeepNullMissingAndLargeHandle(t *testing.T) {
	h := fixtureHandle(t)
	id, err := h.ID()
	if err != nil {
		t.Fatal(err)
	}
	if uint64(id) <= 1<<53 {
		t.Fatal("fixture must use handles beyond JSON number precision")
	}
	params := Params{"null": json.RawMessage(`null`), "handle": json.RawMessage(fmt.Sprintf(`"%d"`, id)), "providerOptions": json.RawMessage(`{"vendor":{"x":null,"ordered":7}}`)}
	result, err := h.Call(context.Background(), "test.echo", params)
	if err != nil {
		t.Fatal(err)
	}
	var got Params
	if err := json.Unmarshal(result, &got); err != nil {
		t.Fatal(err)
	}
	if _, present := got["missing"]; present {
		t.Fatal("absent field became present")
	}
	if string(got["null"]) != "null" || string(got["handle"]) != string(params["handle"]) {
		t.Fatalf("wire changed: %s", result)
	}
	for _, input := range []any{nil, json.RawMessage(`{}`), json.RawMessage(`null`)} {
		result, err := h.Call(context.Background(), "test.echo", input)
		if err != nil {
			t.Fatal(err)
		}
		want, _ := json.Marshal(input)
		if string(result) != string(want) {
			t.Fatalf("normalization: got %s want %s", result, want)
		}
	}
}

func TestCallCancellationBeforeStartAndDuringWait(t *testing.T) {
	h := fixtureHandle(t)
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if _, err := h.Call(ctx, "test.wait", struct{}{}); !errors.Is(err, context.Canceled) {
		t.Fatalf("pre-cancel: %v", err)
	}
	ctx, cancel = context.WithCancel(context.Background())
	defer cancel()
	done := make(chan struct{})
	var err error
	go func() { _, err = h.Call(ctx, "test.wait", struct{}{}); close(done) }()
	awaitNativeWait(t, "waitingCalls")
	cancel()
	await(t, done)
	if !errors.Is(err, context.Canceled) {
		checkCode(t, err, "aimux", 13)
	}
}

func TestCloseCallRace(t *testing.T) {
	fixture(t)
	for i := 0; i < 40; i++ {
		h, err := Create(context.Background(), nil, "test.create", struct{}{})
		if err != nil {
			t.Fatal(err)
		}
		start := make(chan struct{})
		errs := make(chan error, 8)
		var wg sync.WaitGroup
		for j := 0; j < 8; j++ {
			wg.Add(1)
			go func() {
				defer wg.Done()
				<-start
				_, err := h.Call(context.Background(), generated.OP_TEXT_GENERATE, struct{}{})
				errs <- err
			}()
		}
		close(start)
		h.Close()
		h.Close()
		wg.Wait()
		close(errs)
		for err := range errs {
			if err != nil && !errors.Is(err, ErrClosed) {
				checkCode(t, err, "binding", 203)
			}
		}
	}
}

func TestStreamCopiesBorrowedFramesAndLateCancel(t *testing.T) {
	h := fixtureHandle(t)
	s, err := h.Stream(context.Background(), generated.OP_TEXT_STREAM, struct{}{})
	if err != nil {
		t.Fatal(err)
	}
	count := 0
	for event := range s.Events() {
		var payload struct {
			Text string `json:"text"`
		}
		if err := json.Unmarshal(event, &payload); err != nil {
			t.Fatal(err)
		}
		if payload.Text != "copied" {
			t.Fatalf("borrowed callback buffer escaped: %s", event)
		}
		count++
	}
	if count != 1 {
		t.Fatalf("event count = %d", count)
	}
	if err := s.Err(); err != nil {
		t.Fatal(err)
	}
	s.Cancel()
	terminal, err := s.Terminal()
	if err != nil || terminal == nil || terminal.Outcome != "success" {
		t.Fatalf("late cancel rewrote terminal: %+v, %v", terminal, err)
	}
	if n := activeCallbacks.Load(); n != 0 {
		t.Fatalf("cgo handles leaked: %d", n)
	}
}

func TestStreamCancellationFullQueueNoConsumer(t *testing.T) {
	h := fixtureHandle(t)
	for i := 0; i < 40; i++ {
		ctx, cancel := context.WithCancel(context.Background())
		s, err := h.Stream(ctx, "test.flood", struct{}{})
		if err != nil {
			t.Fatal(err)
		}
		// The producer blocks only after filling the bounded output queue.
		deadline := time.Now().Add(time.Second)
		for len(s.events) < cap(s.events) && time.Now().Before(deadline) {
			runtime.Gosched()
		}
		if len(s.events) != cap(s.events) {
			t.Fatal("fixture did not fill queue")
		}
		errDone := make(chan struct{})
		var streamErr error
		go func() { streamErr = s.Err(); close(errDone) }()
		select {
		case <-errDone:
			t.Fatal("Err read producer state before close")
		default:
		}
		if i%2 == 0 {
			cancel()
		} else {
			s.Cancel()
			cancel()
		}
		await(t, s.Done())
		await(t, errDone)
		checkCode(t, streamErr, "aimux", 13)
		terminal, _ := s.Terminal()
		if terminal == nil || terminal.Outcome != "cancelled" {
			t.Fatalf("lost terminal with full queue: %+v", terminal)
		}
		for range s.Events() {
		}
		s.Close()
		if n := activeCallbacks.Load(); n != 0 {
			t.Fatalf("cgo handles leaked: %d", n)
		}
	}
}

func TestStreamPreCancelAndAdmissionRejection(t *testing.T) {
	h := fixtureHandle(t)
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	s, err := h.Stream(ctx, generated.OP_TEXT_STREAM, struct{}{})
	if s != nil || !errors.Is(err, context.Canceled) {
		t.Fatalf("pre-cancel: %v, %v", s, err)
	}
	s, err = h.Stream(context.Background(), "test.reject", struct{}{})
	if s != nil {
		t.Fatal("rejected stream escaped")
	}
	checkCode(t, err, "protocol", 301)
	if activeCallbacks.Load() != 0 {
		t.Fatal("rejection leaked callback context")
	}
}

func TestStreamCancellationDuringStartup(t *testing.T) {
	h := fixtureHandle(t)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan struct{})
	var s *Stream
	var err error
	go func() { s, err = h.Stream(ctx, "test.waitStream", struct{}{}); close(done) }()
	awaitNativeWait(t, "waitingStreams")
	cancel()
	await(t, done)
	if err != nil {
		if !errors.Is(err, context.Canceled) {
			t.Fatal(err)
		}
	} else {
		await(t, s.Done())
		checkCode(t, s.Err(), "aimux", 13)
	}
}

func TestStreamTerminalErrorAndCallbackFailureLatch(t *testing.T) {
	h := fixtureHandle(t)
	for _, c := range []struct {
		op, kind string
		code     int32
	}{
		{"test.terminalError", "aimux", 11}, {"test.bad", "binding", 206},
		{"test.null", "binding", 206}, {"test.large", "protocol", 305},
		{"test.sequence", "binding", 206}, {"test.eventTooLarge", "protocol", 305}, {"test.noend", "binding", 206},
	} {
		t.Run(c.op, func(t *testing.T) {
			s, err := h.Stream(context.Background(), c.op, struct{}{})
			if err == nil {
				for range s.Events() {
				}
				err = s.Err()
			}
			checkCode(t, err, c.kind, c.code)
			if activeCallbacks.Load() != 0 {
				t.Fatal("callback failure leaked context")
			}
		})
	}
}

func TestStreamTerminalCancelRace(t *testing.T) {
	h := fixtureHandle(t)
	for i := 0; i < 100; i++ {
		ctx, cancel := context.WithCancel(context.Background())
		done := make(chan struct{})
		go func() { runtime.Gosched(); cancel(); close(done) }()
		s, err := h.Stream(ctx, generated.OP_TEXT_STREAM, struct{}{})
		if err != nil {
			if !errors.Is(err, context.Canceled) {
				t.Fatal(err)
			}
		} else {
			for range s.Events() {
			}
			terminal, err := s.Terminal()
			if terminal == nil {
				t.Fatal("lost race terminal")
			}
			if terminal.Outcome == "success" {
				if err != nil {
					t.Fatal(err)
				}
			} else {
				checkCode(t, err, "aimux", 13)
			}
		}
		await(t, done)
	}
}

func TestOperationPushPollAndCancellationLifetime(t *testing.T) {
	h := fixtureHandle(t)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	o, err := h.StartOperation(ctx, struct{}{})
	if err != nil {
		t.Fatal(err)
	}
	defer o.Close()
	if err := o.Push(nil); err != nil {
		t.Fatal(err)
	}
	if err := o.Push([]byte("audio")); err != nil {
		t.Fatal(err)
	}
	checkCode(t, o.Push(make([]byte, maxFrameBytes+1)), "protocol", 305)
	result, err := o.NextPart(0)
	if err != nil || result.State != PollTimeout || result.Part != nil {
		t.Fatalf("timeout: %+v, %v", result, err)
	}
	result, err = o.NextPart(time.Second)
	if err != nil || result.State != Part || !json.Valid(result.Part) {
		t.Fatalf("part: %+v, %v", result, err)
	}
	// Unlike a Call-scoped abort, the operation abort still exists here.
	done := make(chan struct{})
	var pushErr error
	go func() { pushErr = o.Push([]byte{255}); close(done) }()
	awaitNativeWait(t, "waitingPushes")
	cancel()
	await(t, done)
	checkCode(t, pushErr, "aimux", 13)
	_, err = o.NextPart(time.Hour)
	checkCode(t, err, "aimux", 13)
	result, err = o.NextPart(0)
	if err != nil || result.State != Ended {
		t.Fatalf("terminal error repeated: %+v, %v", result, err)
	}
	o.Close()
	o.Close()
	if !errors.Is(o.Push(nil), ErrClosed) {
		t.Fatal("push after close")
	}
}

func TestOperationInputDoneAndCancelWithoutConsumer(t *testing.T) {
	h := fixtureHandle(t)
	o, err := h.StartOperation(context.Background(), struct{}{})
	if err != nil {
		t.Fatal(err)
	}
	defer o.Close()
	if err := o.InputDone(context.Background()); err != nil {
		t.Fatal(err)
	}
	if err := o.InputDone(context.Background()); err != nil {
		t.Fatal(err)
	}
	checkCode(t, o.Push([]byte("late")), "protocol", 304)
	result, err := o.NextPart(time.Second)
	if err != nil || result.State != Ended {
		t.Fatalf("inputDone: %+v, %v", result, err)
	}
	o.Cancel()
}

func TestCompositeConstructionPreservesHandleStrings(t *testing.T) {
	child := fixtureHandle(t)
	childID, _ := child.ID()
	params := struct {
		Children []HandleID `json:"children"`
	}{[]HandleID{childID}}
	composite, err := Create(context.Background(), nil, generated.OP_COMPOSITE_ROUTER, params)
	if err != nil {
		t.Fatal(err)
	}
	defer composite.Close()
	child.Close()
	if _, err := composite.Call(context.Background(), generated.OP_TEXT_GENERATE, struct{}{}); err != nil {
		t.Fatal(err)
	}
}

func TestOperationCancellationInterruptsPoll(t *testing.T) {
	h := fixtureHandle(t)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	o, err := h.StartOperation(ctx, struct{}{})
	if err != nil {
		t.Fatal(err)
	}
	defer o.Close()
	if _, err := o.NextPart(time.Second); err != nil {
		t.Fatal(err)
	}
	done := make(chan struct{})
	var pollErr error
	go func() { _, pollErr = o.NextPart(time.Hour); close(done) }()
	awaitNativeWait(t, "waitingPolls")
	cancel()
	await(t, done)
	checkCode(t, pollErr, "aimux", 13)
}

func TestStreamSingleEventLimitIncludesFrameOverhead(t *testing.T) {
	h := fixtureHandle(t)
	s, err := h.Stream(context.Background(), "test.limit", struct{}{})
	if err != nil {
		t.Fatal(err)
	}
	count := 0
	for event := range s.Events() {
		if len(event) != maxItemBytes {
			t.Fatalf("event length = %d", len(event))
		}
		count++
	}
	if err := s.Err(); err != nil {
		t.Fatal(err)
	}
	if count != 1 {
		t.Fatalf("event count = %d", count)
	}
}
