package aimux

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"sync"
	"testing"
	"time"

	"github.com/arcships/aimux/bindings/go/generated"
)

func realRuntime(t *testing.T) {
	t.Helper()
	if os.Getenv("AIMUX_TEST_ABI") == "1" {
		t.Skip("requires the Rust library; run ./test-native.sh")
	}
}

func runtimeProvider(t *testing.T) *Handle {
	t.Helper()
	realRuntime(t)
	// Tests use parameter validation and unsupported methods before network I/O.
	h, err := Create(context.Background(), nil, generated.OP_PROVIDER_CREATE, map[string]any{
		"package":  "openai",
		"settings": map[string]any{"apiKey": "test-key", "baseUrl": "http://example.invalid/v1"},
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { h.Close() })
	return h
}

func runtimeModel(t *testing.T, provider *Handle, method, modelID string) *Handle {
	t.Helper()
	h, err := Create(context.Background(), provider, generated.OP_PROVIDER_MODEL, map[string]any{
		"method": method, "modelId": modelID,
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { h.Close() })
	return h
}

func TestNativeDispatchAndCompositeLifetime(t *testing.T) {
	provider := runtimeProvider(t)
	model := runtimeModel(t, provider, "languageModel", "gpt-4o-mini")
	id, _ := model.ID()
	composite, err := Create(context.Background(), nil, generated.OP_COMPOSITE_ROUTER, map[string]any{
		"children": []HandleID{id}, "strategy": map[string]any{"type": "rule"},
	})
	if err != nil {
		t.Fatal(err)
	}
	defer composite.Close()
	model.Close()
	provider.Close()
	info, err := CallInto[struct{ Modality string }](context.Background(), composite, generated.OP_MODEL_INFO, struct{}{})
	if err != nil || info.Modality != "language" {
		t.Fatalf("retained composite: %+v, %v", info, err)
	}
	_, err = composite.Call(context.Background(), generated.OP_TEXT_GENERATE, struct{}{})
	checkCode(t, err, "protocol", 301)
	_, err = Call(context.Background(), "not.an.operation", struct{}{})
	checkCode(t, err, "protocol", 300)
	_, err = Call(context.Background(), generated.OP_PROVIDER_CREATE, nil)
	checkCode(t, err, "protocol", 301)
	_, err = Call(context.Background(), generated.OP_OPERATION_PUSH, struct{}{})
	checkCode(t, err, "protocol", 300)
}

func TestNativeCloseGenerateRace(t *testing.T) {
	provider := runtimeProvider(t)
	for i := 0; i < 30; i++ {
		model := runtimeModel(t, provider, "languageModel", "gpt-4o-mini")
		start := make(chan struct{})
		var wg sync.WaitGroup
		for j := 0; j < 8; j++ {
			wg.Add(1)
			go func() {
				defer wg.Done()
				<-start
				// Invalid parameters fail before invoking the provider.
				_, err := model.Call(context.Background(), generated.OP_TEXT_GENERATE, struct{}{})
				var native *Error
				if !errors.Is(err, ErrClosed) && (!errors.As(err, &native) || (native.Envelope.Code != 203 && native.Envelope.Code != 301)) {
					t.Errorf("unexpected concurrent call result: %v", err)
				}
			}()
		}
		close(start)
		model.Close()
		model.Close()
		wg.Wait()
	}
}

func TestNativeStreamAdmissionAndTerminal(t *testing.T) {
	provider := runtimeProvider(t)
	model := runtimeModel(t, provider, "languageModel", "gpt-4o-mini")
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if s, err := model.Stream(ctx, generated.OP_TEXT_STREAM, struct{}{}); s != nil || !errors.Is(err, context.Canceled) {
		t.Fatalf("pre-cancel: %v, %v", s, err)
	}
	s, err := model.Stream(context.Background(), generated.OP_TEXT_STREAM, struct{}{})
	if s != nil {
		t.Fatal("invalid stream admitted")
	}
	checkCode(t, err, "protocol", 301)
	s, err = model.Stream(context.Background(), generated.OP_TEXT_STREAM, map[string]any{
		"prompt": []any{}, "options": map[string]any{"maxRetries": 0},
	})
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	for event := range s.Events() {
		if !json.Valid(event) {
			t.Fatalf("invalid event: %s", event)
		}
	}
	terminal, err := s.Terminal()
	if terminal == nil || terminal.Outcome != "error" || err == nil {
		t.Fatalf("provider setup terminal: %+v, %v", terminal, err)
	}
	var native *Error
	if !errors.As(err, &native) || !json.Valid(native.RawEnvelope) {
		t.Fatalf("lost native error: %v", err)
	}
	s.Cancel()
	if s.Err() != err {
		t.Fatal("late cancellation changed terminal error")
	}
	if activeCallbacks.Load() != 0 {
		t.Fatal("native callback handle leaked")
	}
}

func TestNativeOperationTerminalPersistence(t *testing.T) {
	provider := runtimeProvider(t)
	// Non-realtime models return UnsupportedFunctionality without connecting.
	model := runtimeModel(t, provider, "transcriptionModel", "whisper-1")
	o, err := model.StartOperation(context.Background(), struct{}{})
	if err != nil {
		t.Fatal(err)
	}
	defer o.Close()
	_, err = o.NextPart(-time.Millisecond)
	checkCode(t, err, "protocol", 301)
	_, err = o.NextPart(5 * time.Second)
	terminalError := checkCode(t, err, "aimux", 8)
	result, err := o.NextPart(0)
	if err != nil || result.State != Ended || result.Part != nil {
		t.Fatalf("ended poll: %+v, %v", result, err)
	}
	status, err := CallInto[struct {
		State    string
		Terminal struct{ Error *ErrorEnvelope }
	}](context.Background(), o.handle, generated.OP_OPERATION_STATUS, struct{}{})
	if err != nil {
		t.Fatal(err)
	}
	if status.State != "terminal" || status.Terminal.Error == nil || status.Terminal.Error.Code != terminalError.Envelope.Code {
		t.Fatalf("persistent terminal: %+v", status)
	}
	o.Cancel()
	o.Close()
	o.Close()
	if !errors.Is(o.Push(nil), ErrClosed) {
		t.Fatal("push after operation close")
	}
}
