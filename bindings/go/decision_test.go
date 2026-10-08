package aimux

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"reflect"
	"testing"
	"time"
)

func TestProviderDecisionModelOwnsHandle(t *testing.T) {
	provider, err := CreateProvider("openai", "test-key", nil)
	if err != nil {
		t.Fatal(err)
	}
	defer provider.Close()
	model, err := provider.DecisionModel("gpt-6-luna")
	if err != nil {
		t.Fatal(err)
	}
	defer model.Close()
	provider.Close()
	caps, err := model.Capabilities()
	if err != nil {
		t.Fatal(err)
	}
	var decoded map[string]any
	if err := json.Unmarshal([]byte(caps), &decoded); err != nil {
		t.Fatal(err)
	}
	if decoded["min_choices"] != float64(2) {
		t.Fatal(caps)
	}
	if _, err := provider.DecisionModel("gpt-6-luna"); !errors.Is(err, ErrClosed) {
		t.Fatalf("closed provider: %v", err)
	}
}

func TestDecisionNativeContract(t *testing.T) {
	data, err := os.ReadFile("../../contract-tests/fixtures/decision-native.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture map[string]json.RawMessage
	if err = json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	requests := make(chan map[string]any, 1)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/systemone" || r.Header.Get("Authorization") != "Bearer test-key" {
			t.Errorf("unexpected request path/auth: %s", r.URL.Path)
		}
		var request map[string]any
		if err := json.NewDecoder(r.Body).Decode(&request); err != nil {
			t.Error(err)
		}
		requests <- request
		w.Header().Set("Content-Type", "application/json")
		w.Write(fixture["response"])
	}))
	defer server.Close()
	model, err := NewJevDecisionWithEndpoint("test-key", "jev-latest", server.URL+"/v1/systemone", "native")
	if err != nil {
		t.Fatal(err)
	}
	defer model.Close()
	capabilities, err := model.Capabilities()
	if err != nil {
		t.Fatal(err)
	}
	var caps map[string]any
	json.Unmarshal([]byte(capabilities), &caps)
	if caps["max_choices"] != float64(255) || caps["rounding"].(map[string]any)["probability_decimals"] != float64(2) {
		t.Fatalf("unexpected capabilities: %s", capabilities)
	}
	if len(requests) != 0 {
		t.Fatal("capability query made an HTTP request")
	}
	result, err := model.DecideJSONContext(context.Background(), string(fixture["request"]))
	if err != nil {
		t.Fatal(err)
	}
	var decoded map[string]any
	json.Unmarshal([]byte(result), &decoded)
	answers := decoded["answers"].(map[string]any)
	if answers["urgent"].(map[string]any)["probability_true"] != 0.9 {
		t.Fatal(result)
	}
	var original map[string]any
	json.Unmarshal(fixture["request"], &original)
	questions := original["questions"].([]any)
	wire := (<-requests)["questions"].(map[string]any)
	if !reflect.DeepEqual(wire["urgent"].(map[string]any)["criteria"], questions[0].(map[string]any)["criteria"]) {
		t.Fatal("boolean criteria changed")
	}
	if !reflect.DeepEqual(answers["severity"].(map[string]any)["levels"], questions[2].(map[string]any)["levels"]) {
		t.Fatal("structured score levels changed")
	}
	model.Close()
	if _, err := model.Capabilities(); !errors.Is(err, ErrClosed) {
		t.Fatalf("closed: %v", err)
	}
	if _, err := NewJevDecisionWithEndpoint("test-key", "jev-latest", "", "unknown"); err == nil {
		t.Fatal("unknown provenance accepted")
	}
}

func TestDecisionContextCancelsNativeRequest(t *testing.T) {
	reached := make(chan struct{})
	released := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		close(reached)
		<-released
	}))
	defer server.Close()
	defer close(released)
	model, err := NewJevDecisionWithEndpoint("test", "jev-latest", server.URL+"/v1/systemone", "")
	if err != nil {
		t.Fatal(err)
	}
	defer model.Close()
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	result := make(chan error, 1)
	go func() {
		_, err := model.DecideContext(ctx, map[string]any{"state": "test", "questions": []any{
			map[string]any{"id": "q", "type": "boolean", "instructions": "Urgent?"}}})
		result <- err
	}()
	select {
	case <-reached:
	case <-time.After(3 * time.Second):
		t.Fatal("request did not start")
	}
	cancel()
	select {
	case err := <-result:
		if !errors.Is(err, context.Canceled) {
			t.Fatal(err)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("cancellation hung")
	}
}
