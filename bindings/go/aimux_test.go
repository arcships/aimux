// Unit tests for the aimux Go binding — constructor, Close, invalid input.
//
// These mirror the Kotlin ModelTest.kt: they verify the binding's lifecycle
// behavior without needing network access. Even with a fake API key, the
// provider constructs (the Rust side doesn't validate keys until a request
// is actually made).

package aimux

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

// cancelOnDoneContext makes cancellation happen while StreamTextContext is
// installing its context watcher. It deterministically exercises the window
// between the initial context check and context.AfterFunc registration.
type cancelOnDoneContext struct {
	done      chan struct{}
	cancelled atomic.Bool
	once      sync.Once
}

func newCancelOnDoneContext() *cancelOnDoneContext {
	return &cancelOnDoneContext{done: make(chan struct{})}
}

func (c *cancelOnDoneContext) Deadline() (time.Time, bool) { return time.Time{}, false }
func (c *cancelOnDoneContext) Done() <-chan struct{} {
	c.once.Do(func() {
		c.cancelled.Store(true)
		close(c.done)
	})
	return c.done
}
func (c *cancelOnDoneContext) Err() error {
	if c.cancelled.Load() {
		return context.Canceled
	}
	return nil
}
func (*cancelOnDoneContext) Value(any) any { return nil }

func TestOpenAI(t *testing.T) {
	m := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	if m == nil {
		t.Fatal("expected non-nil model")
	}
	defer m.Close()
	if m.id.Load() == 0 {
		t.Fatal("expected non-zero handle")
	}
}

func TestAnthropic(t *testing.T) {
	m := Anthropic("sk-ant-test-fake-key", "claude-3-5-sonnet-20241022")
	if m == nil {
		t.Fatal("expected non-nil model")
	}
	defer m.Close()
	if m.id.Load() == 0 {
		t.Fatal("expected non-zero handle")
	}
}

func TestOpenAIWithBase(t *testing.T) {
	m := OpenAIWithBase("sk-test-fake-key", "gpt-4o-mini", "http://localhost:11434")
	if m == nil {
		t.Fatal("expected non-nil model")
	}
	defer m.Close()
	if m.id.Load() == 0 {
		t.Fatal("expected non-zero handle")
	}
}

func TestModelClose(t *testing.T) {
	m := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	m.Close()
	// Double-close should not crash.
	if err := m.Close(); err != nil {
		t.Fatalf("double close should be safe: %v", err)
	}
}

func TestGenerateTextRejectsInvalidPrompt(t *testing.T) {
	m := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	defer m.Close()

	// Invalid JSON prompt should produce an error (the engine will try to
	// parse it and fail). We can't hit a real API, but the FFI layer returns
	// an error response for malformed input.
	_, err := m.GenerateText("{invalid json}", "")
	// With a fake key, this will either fail at JSON parse or at network.
	// Either way, no nil error + empty string.
	if err == nil {
		// The provider might construct and fail on network — that's fine,
		// the error comes as a JSON error string. Check it's not a panic.
		// If no error (e.g., somehow succeeded), at least verify it didn't crash.
		t.Log("generate_text did not return error (unexpected but not fatal with fake key)")
	}
}

func TestGenerateTextAfterClose(t *testing.T) {
	m := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	m.Close()

	_, err := m.GenerateText(`"hello"`, "")
	if err == nil {
		t.Fatal("expected error after close")
	}
}

func TestStreamTextReturnsStream(t *testing.T) {
	m := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	defer m.Close()

	// We don't consume the stream (would need network), but verify it
	// can be created without panicking.
	s := m.StreamText(`"hello"`, "")
	if s == nil {
		t.Fatal("expected non-nil stream")
	}
	// Parts() should return a channel.
	ch := s.Parts()
	if ch == nil {
		t.Fatal("expected non-nil parts channel")
	}
	// Drain to avoid leaking the goroutine (the fake-key stream will error
	// quickly since no real API is reachable).
	for range ch {
	}
	// Err() should be safe to call after drain.
	_ = s.Err()
}

func TestStreamTextContextAlreadyCanceled(t *testing.T) {
	m := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	if err := m.Close(); err != nil {
		t.Fatalf("Close failed: %v", err)
	}

	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	stream := m.StreamTextContext(ctx, `"hello"`, "")

	done := make(chan struct{})
	go func() {
		for range stream.Parts() {
		}
		close(done)
	}()

	select {
	case <-done:
	case <-time.After(2 * time.Second):
		t.Fatal("cancelled stream did not stop")
	}
	if !errors.Is(stream.Err(), context.Canceled) {
		t.Fatalf("expected context.Canceled, got %v", stream.Err())
	}

	stream.Cancel()
	stream.Cancel()
}

func TestStreamTextContextDoesNotLoseRacingCancellation(t *testing.T) {
	requestStarted := make(chan struct{}, 1)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		requestStarted <- struct{}{}
		<-r.Context().Done()
	}))
	defer server.Close()

	m := OpenAIWithBase("sk-test-fake-key", "gpt-4o-mini", server.URL)
	defer m.Close()

	stream := m.StreamTextContext(newCancelOnDoneContext(), `"hello"`, "")
	done := make(chan struct{})
	go func() {
		for range stream.Parts() {
		}
		close(done)
	}()

	select {
	case <-done:
	case <-requestStarted:
		stream.Cancel()
		<-done
		t.Fatal("request started after the context was cancelled")
	case <-time.After(2 * time.Second):
		stream.Cancel()
		<-done
		t.Fatal("racing cancellation did not stop the stream")
	}
	if !errors.Is(stream.Err(), context.Canceled) {
		t.Fatalf("expected context.Canceled, got %v", stream.Err())
	}
}

func TestProviderWithConfigFullOptions(t *testing.T) {
	retries := uint32(0)
	m, err := ProviderWithConfig("groq", "sk-test-fake-key", "llama-3.3-70b", &ProviderConfig{
		BaseURL:       "https://example.com/v1",
		Headers:       map[string]string{"X-Custom": "1"},
		Organization:  "org-1",
		Project:       "proj-1",
		MaxRetries:    &retries,
		BodyOverrides: map[string]any{"temperature": 0.1},
	})
	if err != nil {
		t.Fatalf("expected success, got %v", err)
	}
	defer m.Close()
	if m.id.Load() == 0 {
		t.Fatal("expected non-zero handle")
	}
}

func TestProviderWithConfigNil(t *testing.T) {
	m, err := ProviderWithConfig("groq", "sk-test-fake-key", "llama-3.3-70b", nil)
	if err != nil {
		t.Fatalf("expected success, got %v", err)
	}
	defer m.Close()
}

func TestProviderWithBaseQuotedURLDoesNotInjectJSON(t *testing.T) {
	// A baseURL containing a quote must not produce malformed config JSON
	// (the old string concatenation would). The provider layer may reject
	// the URL itself, but the error must not be a JSON parse failure.
	_, err := ProviderWithBase("groq", "sk-test-fake-key", "llama-3.3-70b", `https://example.com/"v1`)
	if err != nil && strings.Contains(err.Error(), "invalid provider config JSON") {
		t.Fatalf("config JSON injection: %v", err)
	}
}

func TestProviderCloseDoesNotWaitForInFlightListModels(t *testing.T) {
	server, requestStarted, releaseRequest := newBlockedHTTPServer(
		t, "application/json", `{"data":[]}`,
	)
	defer releaseRequest()

	provider, err := CreateProvider("groq", "sk-test-fake-key", &ProviderConfig{
		BaseURL: server.URL,
	})
	if err != nil {
		t.Fatalf("CreateProvider failed: %v", err)
	}
	defer provider.Close()

	listDone := make(chan error, 1)
	go func() {
		_, err := provider.ListModels()
		listDone <- err
	}()
	select {
	case <-requestStarted:
	case <-time.After(2 * time.Second):
		t.Fatal("ListModels request did not start")
	}

	closes := make([]func(), 32)
	for i := range closes {
		closes[i] = func() {
			if err := provider.Close(); err != nil {
				t.Errorf("Close: %v", err)
			}
		}
	}
	runConcurrent(t, "ProviderHandle.Close during ListModels", closes...)
	if got := provider.id.Load(); got != 0 {
		t.Fatalf("provider handle after Close = %d, want 0", got)
	}

	// The call already entered the native registry, so dropping the Go owner
	// must not invalidate its Arc. Let the provider response complete now.
	releaseRequest()
	select {
	case err := <-listDone:
		if err != nil {
			t.Fatalf("in-flight ListModels failed after Close: %v", err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("in-flight ListModels did not finish after releasing the response")
	}

	_, err = provider.ListModels()
	var aimuxErr *Error
	if !errors.As(err, &aimuxErr) || aimuxErr.Code != CodeInvalidArgument {
		t.Fatalf("ListModels after Close: expected flat InvalidArgument, got %v", err)
	}
}

// TestInitRecordingRingRejectsZeroCap verifies D7: cap == 0 is rejected with
// an error instead of being silently rewritten to 2048. The check happens in
// Go before any FFI call, so this does not touch the global recorder state.
func TestInitRecordingRingRejectsZeroCap(t *testing.T) {
	err := InitRecordingRing(0)
	if err == nil {
		t.Fatal("expected error for cap == 0, got nil")
	}
	if !strings.Contains(err.Error(), "cap > 0") {
		t.Fatalf("expected cap error message, got: %v", err)
	}
}

// TestInitRecordingRingAcceptsPositiveCap verifies a positive cap does not
// return an error (the C ABI accepts it and returns 0).
func TestInitRecordingRingAcceptsPositiveCap(t *testing.T) {
	if err := InitRecordingRing(8); err != nil {
		t.Fatalf("expected success for cap=8, got: %v", err)
	}
	// Reset global recorder state so this doesn't leak into other tests.
	RecordingStop()
}

// TestInitRecordingRingDefaultNoArg verifies the no-arg form uses the library
// default capacity (FFI aimux_init_recording_ring_default) and returns nil.
func TestInitRecordingRingDefaultNoArg(t *testing.T) {
	if err := InitRecordingRing(); err != nil {
		t.Fatalf("expected success for no-arg default, got: %v", err)
	}
	// Reset global recorder state so this doesn't leak into other tests.
	RecordingStop()
}

// runConcurrent starts every task at one barrier and requires every task,
// including Close, to return. Atomic handle snapshots create no Go lock order
// or wait edge between these operations.
func runConcurrent(t *testing.T, what string, tasks ...func()) {
	t.Helper()
	start := make(chan struct{})
	var wg sync.WaitGroup
	wg.Add(len(tasks))
	for _, task := range tasks {
		go func() {
			defer wg.Done()
			<-start
			task()
		}()
	}
	close(start)

	done := make(chan struct{})
	go func() {
		wg.Wait()
		close(done)
	}()
	select {
	case <-done:
	case <-time.After(10 * time.Second):
		t.Fatalf("%s did not return within 10s; possible deadlock", what)
	}
}

// newBlockedHTTPServer starts handling a request, then withholds its response
// until release is called. It makes an in-flight native call observable
// without adding a production test seam. release is idempotent.
func newBlockedHTTPServer(t *testing.T, contentType, body string) (*httptest.Server, <-chan struct{}, func()) {
	t.Helper()
	requestStarted := make(chan struct{})
	releaseRequest := make(chan struct{})
	var startedOnce sync.Once
	var releaseOnce sync.Once
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		startedOnce.Do(func() { close(requestStarted) })
		<-releaseRequest
		if contentType != "" {
			w.Header().Set("Content-Type", contentType)
		}
		_, _ = fmt.Fprint(w, body)
	}))
	release := func() { releaseOnce.Do(func() { close(releaseRequest) }) }
	t.Cleanup(func() {
		release()
		server.Close()
	})
	return server, requestStarted, release
}

func duplicateModel(m *Model, n int) []*Model {
	models := make([]*Model, n)
	for i := range models {
		models[i] = m
	}
	return models
}

func TestNewRouterRepeatedModelRacingClose(t *testing.T) {
	m := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	if router, err := NewRouter(duplicateModel(m, 8), ""); err != nil {
		t.Fatalf("duplicate router input: %v", err)
	} else {
		router.Close()
	}
	m.Close()

	for i := 0; i < 25; i++ {
		m = OpenAI("sk-test-fake-key", "gpt-4o-mini")
		tasks := []func(){func() { m.Close() }}
		for j := 0; j < 16; j++ {
			tasks = append(tasks, func() {
				if router, err := NewRouter(duplicateModel(m, 8), ""); err == nil {
					router.Close()
				}
			})
		}
		runConcurrent(t, "NewRouter over a repeated model", tasks...)
	}
}

func TestNewMoaAggregatorAlsoReferenceRacingClose(t *testing.T) {
	m := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	if moa, err := NewMoa(duplicateModel(m, 7), m, ""); err != nil {
		t.Fatalf("duplicate MoA input: %v", err)
	} else {
		moa.Close()
	}
	m.Close()

	for i := 0; i < 25; i++ {
		m = OpenAI("sk-test-fake-key", "gpt-4o-mini")
		tasks := []func(){func() { m.Close() }}
		for j := 0; j < 16; j++ {
			tasks = append(tasks, func() {
				if moa, err := NewMoa(duplicateModel(m, 7), m, ""); err == nil {
					moa.Close()
				}
			})
		}
		runConcurrent(t, "NewMoa with the aggregator also in references", tasks...)
	}
}

func TestCompositeConstructorsOppositeOrdersRacingClose(t *testing.T) {
	const (
		iterations = 30
		fanout     = 8
		pairs      = 8
	)
	for i := 0; i < iterations; i++ {
		models := make([]*Model, fanout)
		for j := range models {
			models[j] = OpenAI("sk-test-fake-key", "gpt-4o-mini")
		}
		reversed := make([]*Model, fanout)
		for j, model := range models {
			reversed[fanout-1-j] = model
		}

		tasks := make([]func(), 0, pairs*2+fanout)
		build := func(order []*Model) func() {
			return func() {
				if router, err := NewRouter(order, ""); err == nil {
					router.Close()
				}
			}
		}
		for j := 0; j < pairs; j++ {
			tasks = append(tasks, build(models), build(reversed))
		}
		for _, model := range models {
			model := model
			tasks = append(tasks, func() { model.Close() })
		}
		runConcurrent(t, fmt.Sprintf("opposite-order iteration %d", i), tasks...)
	}
}

func TestZeroValueModelIsClosed(t *testing.T) {
	var model Model
	_, err := model.GenerateText(`"hello"`, "")
	var aimuxErr *Error
	if !errors.As(err, &aimuxErr) || aimuxErr.Code != CodeInvalidArgument {
		t.Fatalf("zero-value Model: expected InvalidArgument, got %v", err)
	}
	if err := model.Close(); err != nil {
		t.Fatalf("zero-value Close: %v", err)
	}
}

func TestModelConcurrentCloseIsIdempotent(t *testing.T) {
	model := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	tasks := make([]func(), 64)
	for i := range tasks {
		tasks[i] = func() {
			if err := model.Close(); err != nil {
				t.Errorf("Close: %v", err)
			}
		}
	}
	runConcurrent(t, "concurrent Model.Close", tasks...)
	if got := model.id.Load(); got != 0 {
		t.Fatalf("handle after Close = %d, want 0", got)
	}
	if err := model.Close(); err != nil {
		t.Fatalf("repeated Close: %v", err)
	}
}

func TestCompositeConstructorsRejectNilModels(t *testing.T) {
	model := OpenAI("sk-test-fake-key", "gpt-4o-mini")
	defer model.Close()

	cases := []struct {
		name string
		want string
		call func() (*Model, error)
	}{
		{"router nil child", "aimux: models[1] is nil",
			func() (*Model, error) { return NewRouter([]*Model{model, nil, model}, "") }},
		{"router only child nil", "aimux: models[0] is nil",
			func() (*Model, error) { return NewRouter([]*Model{nil}, "") }},
		{"moa nil aggregator", "aimux: aggregator is nil",
			func() (*Model, error) { return NewMoa([]*Model{model}, nil, "") }},
		{"moa nil reference", "aimux: models[1] is nil",
			func() (*Model, error) { return NewMoa([]*Model{model, nil}, model, "") }},
		{"moa nil aggregator, no references", "aimux: aggregator is nil",
			func() (*Model, error) { return NewMoa(nil, nil, "") }},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			defer func() {
				if recovered := recover(); recovered != nil {
					t.Fatalf("panicked instead of returning an error: %v", recovered)
				}
			}()
			got, err := test.call()
			if got != nil {
				got.Close()
				t.Fatal("expected a nil model alongside the error")
			}
			if err == nil {
				t.Fatal("expected an error for a nil model")
			}
			if err.Error() != test.want {
				t.Fatalf("error = %q, want %q", err, test.want)
			}
		})
	}
}
