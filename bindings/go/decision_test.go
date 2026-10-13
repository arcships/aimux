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
	description, err := model.Describe()
	if err != nil {
		t.Fatal(err)
	}
	var decoded map[string]any
	if err := json.Unmarshal([]byte(description), &decoded); err != nil {
		t.Fatal(err)
	}
	if decoded["specification_version"] != "v4" || decoded["provider"] != "openai" || decoded["model_id"] != "gpt-6-luna" {
		t.Fatal(description)
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
	description, err := model.Describe()
	if err != nil {
		t.Fatal(err)
	}
	var properties map[string]any
	json.Unmarshal([]byte(description), &properties)
	if !reflect.DeepEqual(properties["supported_question_types"], []any{"choice", "score", "boolean"}) || properties["provider"] != "jev" {
		t.Fatalf("unexpected description: %s", description)
	}
	if len(requests) != 0 {
		t.Fatal("describe made an HTTP request")
	}
	result, err := model.DecideJSONContext(context.Background(), string(fixture["request"]))
	if err != nil {
		t.Fatal(err)
	}
	var decoded map[string]any
	json.Unmarshal([]byte(result), &decoded)
	answers := decoded["answers"].(map[string]any)
	if answers["urgent"].(map[string]any)["probability"] != 0.9 {
		t.Fatal(result)
	}
	if decoded["rounding"].(map[string]any)["probability_decimals"] != float64(2) {
		t.Fatal(result)
	}
	var original map[string]any
	json.Unmarshal(fixture["request"], &original)
	questions := original["questions"].(map[string]any)
	wire := (<-requests)["questions"].(map[string]any)
	if !reflect.DeepEqual(wire["urgent"].(map[string]any)["criteria"], questions["urgent"].(map[string]any)["criteria"]) {
		t.Fatal("boolean criteria changed")
	}
	if !reflect.DeepEqual(wire["severity"].(map[string]any)["criteria"], questions["severity"].(map[string]any)["criteria"]) {
		t.Fatal("structured score levels changed")
	}
	model.Close()
	if _, err := model.Describe(); !errors.Is(err, ErrClosed) {
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
		_, err := model.DecideContext(ctx, map[string]any{"state": "test", "questions": map[string]any{
			"q": map[string]any{"type": "boolean", "instructions": "Urgent?"}}})
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

func TestOpenAIDecisionMediaAndNativeValues(t *testing.T) {
	data, err := os.ReadFile("../../contract-tests/fixtures/decision-openai-full.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture map[string]json.RawMessage
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	requests := make(chan map[string]any, 1)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/decisions" {
			t.Errorf("wrong decision route: %s", r.URL.Path)
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
	provider, err := CreateProvider("openai", "test-key", &ProviderConfig{BaseURL: server.URL + "/v1"})
	if err != nil {
		t.Fatal(err)
	}
	defer provider.Close()
	model, err := provider.DecisionModel("gpt-6-luna")
	if err != nil {
		t.Fatal(err)
	}
	defer model.Close()
	result, err := model.DecideJSONContext(context.Background(), string(fixture["options"]))
	if err != nil {
		t.Fatal(err)
	}
	var decoded, expected map[string]any
	json.Unmarshal([]byte(result), &decoded)
	json.Unmarshal(fixture["request"], &expected)
	if !reflect.DeepEqual(<-requests, expected) {
		t.Fatal("native request changed across Go boundary")
	}
	choice := decoded["answers"].(map[string]any)["choice"].(map[string]any)
	if choice["choice"] != "boolean_true" {
		t.Fatal(result)
	}
	metadata := decoded["provider_metadata"].(map[string]any)["openai"].(map[string]any)
	if metadata["answers"].(map[string]any)["choice"].(map[string]any)["value"] != true {
		t.Fatal(result)
	}
}
