// Package aimux provides Go bindings for the aimux unified LLM service layer
// (325 AI providers via a single API).
//
// This is the C ABI path (RFC-0001 §3.2) — same as Swift/Kotlin/Flutter/C.
// Go calls aimux-ffi via cgo, statically linking libaimux_ffi.a. The result is
// a single binary with the Rust core compiled in (no .so/.dll/.dylib to ship).
//
// Design doc: rfc/0011-golang-bindings.md
package aimux

/*
#cgo CFLAGS: -I${SRCDIR}/../../aimux-ffi
#cgo linux LDFLAGS: -L${SRCDIR}/../../target/release -Wl,-Bstatic -laimux_ffi -Wl,-Bdynamic -lpthread -ldl -lm
#cgo darwin LDFLAGS: -L${SRCDIR}/../../target/release -laimux_ffi
#cgo windows LDFLAGS: -L${SRCDIR}/../../target/release -Wl,-Bstatic -laimux_ffi -Wl,-Bdynamic -lws2_32 -lbcrypt -lntdll -luserenv

#include <stdint.h>
#include <stdlib.h>

#include "aimux-ffi.h"

// Go-side callback trampolines (//export below). stream_ctx is the stream id.
extern void goStreamPart(int64_t id, char* json);
extern void goStreamDone(int64_t id);

static void trampoline_part(const char* json, void* stream_ctx) {
    int64_t id = (int64_t)(intptr_t)stream_ctx;
    if (id) goStreamPart(id, (char*)json);
}
static void trampoline_done(void* stream_ctx) {
    int64_t id = (int64_t)(intptr_t)stream_ctx;
    if (id) goStreamDone(id);
}

// do_stream: blocking cancelable stream; id is passed as stream_ctx.
// On failure, *err is filled and return is 0 (no on_done).
static int32_t do_stream(uint64_t handle, uint64_t abort_handle,
                         const char* prompt, const char* opts, int64_t id,
                         AimuxError* err) {
    return aimux_stream_text_with_abort(
        handle, abort_handle, prompt, opts,
        trampoline_part, trampoline_done,
        (void*)(intptr_t)id, err);
}

static int32_t do_stream_openai(uint64_t handle, uint64_t abort_handle,
                                const char* prompt, const char* opts, int64_t id,
                                AimuxError* err) {
    return aimux_stream_text_as_openai_with_abort(
        handle, abort_handle, prompt, opts,
        trampoline_part, trampoline_done,
        (void*)(intptr_t)id, err);
}

*/
import "C"

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"runtime"
	"sync"
	"unsafe"
)

// ── Model handle wrapper ───────────────────────────────────────────────────

// Model is a model instance backed by a Rust Arc<dyn LanguageModel>.
//
// It implements io.Closer — you MUST call Close (or use defer) to release the
// native handle and avoid memory leaks.
//
// Concurrency: Model is safe for concurrent use. GenerateText and StreamText
// acquire a read lock; Close acquires a write lock and waits for in-flight
// calls to finish before dropping the native handle.
type Model struct {
	mu     sync.RWMutex
	handle uint64
	closed bool
}

// Close releases the native handle. Safe to call multiple times.
// It blocks until in-flight GenerateText/StreamText calls finish.
func (m *Model) Close() error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.closed {
		return nil
	}
	m.closed = true
	if m.handle != 0 {
		C.aimux_drop_handle(C.uint64_t(m.handle))
		m.handle = 0
	}
	runtime.SetFinalizer(m, nil)
	return nil
}

// acquireHandle returns the native handle under a read lock, or an error if
// the model is closed. The caller must call the returned release func when
// done with the handle (deferred after the FFI call returns).
func (m *Model) acquireHandle() (uint64, func(), error) {
	m.mu.RLock()
	if m.closed {
		m.mu.RUnlock()
		return 0, nil, newError(CodeInvalidArgument, "aimux: model already closed")
	}
	h := m.handle
	return h, m.mu.RUnlock, nil
}

// ── Provider constructors ───────────────────────────────────────────────────

// NewOpenAI creates an OpenAI model instance.
//
//	m, err := aimux.NewOpenAI("sk-...", "gpt-4o")
//	if err != nil { ... }
//	defer m.Close()
func NewOpenAI(apiKey, modelID string) (*Model, error) {
	return newModel(apiKey, modelID, "", "openai")
}

// NewOpenAIWithBase creates an OpenAI model with a custom base URL
// (for Ollama, OpenRouter, local proxies, etc.).
func NewOpenAIWithBase(apiKey, modelID, baseURL string) (*Model, error) {
	return newModel(apiKey, modelID, baseURL, "openai")
}

// NewAnthropic creates an Anthropic model instance.
func NewAnthropic(apiKey, modelID string) (*Model, error) {
	return newModel(apiKey, modelID, "", "anthropic")
}

// NewAnthropicWithBase creates an Anthropic model with a custom base URL.
func NewAnthropicWithBase(apiKey, modelID, baseURL string) (*Model, error) {
	return newModel(apiKey, modelID, baseURL, "anthropic")
}

// NewCohere creates a Cohere model instance.
func NewCohere(apiKey, modelID string) (*Model, error) {
	return newModel(apiKey, modelID, "", "cohere")
}

// NewCohereWithBase creates a Cohere model with a custom base URL.
func NewCohereWithBase(apiKey, modelID, baseURL string) (*Model, error) {
	return newModel(apiKey, modelID, baseURL, "cohere")
}

// NewMistral creates a Mistral model instance.
func NewMistral(apiKey, modelID string) (*Model, error) {
	return newModel(apiKey, modelID, "", "mistral")
}

// NewMistralWithBase creates a Mistral model with a custom base URL.
func NewMistralWithBase(apiKey, modelID, baseURL string) (*Model, error) {
	return newModel(apiKey, modelID, baseURL, "mistral")
}

// NewXai creates an xAI model instance.
func NewXai(apiKey, modelID string) (*Model, error) {
	return newModel(apiKey, modelID, "", "xai")
}

// NewXaiWithBase creates an xAI model with a custom base URL.
func NewXaiWithBase(apiKey, modelID, baseURL string) (*Model, error) {
	return newModel(apiKey, modelID, baseURL, "xai")
}

// NewBedrock creates a Bedrock model instance (AWS SigV4 credentials).
func NewBedrock(accessKeyID, secretAccessKey, region, modelID string) (*Model, error) {
	return newBedrockModel(accessKeyID, secretAccessKey, region, modelID, "")
}

// NewBedrockWithBase creates a Bedrock model with a custom base URL.
func NewBedrockWithBase(accessKeyID, secretAccessKey, region, modelID, baseURL string) (*Model, error) {
	return newBedrockModel(accessKeyID, secretAccessKey, region, modelID, baseURL)
}

// NewVertex creates a Vertex AI model instance (GCP bearer token).
func NewVertex(accessToken, project, location, modelID string) (*Model, error) {
	return newVertexModel(accessToken, project, location, modelID, "")
}

// NewVertexWithBase creates a Vertex AI model with a custom base URL.
func NewVertexWithBase(accessToken, project, location, modelID, baseURL string) (*Model, error) {
	return newVertexModel(accessToken, project, location, modelID, baseURL)
}

// NewAnthropicAws creates an Anthropic-on-AWS model instance (API key + region).
func NewAnthropicAws(apiKey, region, modelID string) (*Model, error) {
	return newAnthropicAwsModel(apiKey, region, modelID, "")
}

// NewAnthropicAwsWithBase creates an Anthropic-on-AWS model with a custom base URL.
func NewAnthropicAwsWithBase(apiKey, region, modelID, baseURL string) (*Model, error) {
	return newAnthropicAwsModel(apiKey, region, modelID, baseURL)
}

// NewAzure creates an Azure OpenAI model instance (API key + resource name).
// The deployment name is passed as modelID; apiVersion "" uses the default.
func NewAzure(apiKey, resourceName, deployment string) (*Model, error) {
	return newAzureModel(apiKey, resourceName, deployment, "", false)
}

// NewAzureWithVersion creates an Azure OpenAI model with an explicit api-version.
func NewAzureWithVersion(apiKey, resourceName, deployment, apiVersion string) (*Model, error) {
	return newAzureModel(apiKey, resourceName, deployment, apiVersion, false)
}

// NewAzureWithBase creates an Azure OpenAI model with a custom base URL.
func NewAzureWithBase(apiKey, baseURL, deployment string) (*Model, error) {
	return newAzureModel(apiKey, baseURL, deployment, "", true)
}

// OpenAI creates an OpenAI model instance, panicking on failure.
// Prefer NewOpenAI for explicit error handling.
func OpenAI(apiKey, modelID string) *Model {
	return mustNew(NewOpenAI(apiKey, modelID))
}

// OpenAIWithBase creates an OpenAI model with a custom base URL, panicking on failure.
func OpenAIWithBase(apiKey, modelID, baseURL string) *Model {
	return mustNew(NewOpenAIWithBase(apiKey, modelID, baseURL))
}

// InitLogging initializes the global logger (RFC-0014). Idempotent — safe to
// call any number of times; no-op if the host already registered its own
// subscriber. level: "off"|"error"|"warn"|"info"|"debug"|"trace" (empty
// defaults to "warn"). The AIMUX_LOG / AIMUX_LOG_LEVEL env vars take
// precedence. Logs go to stderr.
func InitLogging(level string) {
	if level == "" {
		level = "warn"
	}
	cLevel := C.CString(level)
	defer C.free(unsafe.Pointer(cLevel))
	C.aimux_init_logging(cLevel)
}

// InitRecording starts RFC-0023 recording: complete Recording JSONL is written
// to {dir}/recordings.jsonl (dir auto-created). Recording is opt-in; calling
// again replaces the recorder.
func InitRecording(dir string) {
	cDir := C.CString(dir)
	defer C.free(unsafe.Pointer(cDir))
	C.aimux_init_recording(cDir)
}

// InitRecordingRing starts in-memory bounded recording (RFC-0023 P6): FIFO
// ring with the given capacity.
//
// cap is optional (variadic): omit it to use the library default capacity,
// which calls the FFI aimux_init_recording_ring_default entry point. When
// provided, cap must be > 0 — the C ABI rejects cap == 0 (returns -1). Unlike
// the previous behavior, this binding no longer silently rewrites cap == 0 to
// 2048; callers must omit cap for the default or pass an explicit > 0
// capacity. This matches Kotlin/Java (which throw) and Swift/Flutter (which
// surface the C error). Returns an error when cap == 0, more than one cap is
// supplied, or the C call fails.
func InitRecordingRing(cap ...uint64) error {
	if len(cap) == 0 {
		// No cap: library default capacity (FFI default entry point).
		rc := C.aimux_init_recording_ring_default()
		if rc != 0 {
			return fmt.Errorf("aimux: InitRecordingRing default failed (rc=%d)", int32(rc))
		}
		return nil
	}
	if len(cap) > 1 {
		return fmt.Errorf("aimux: InitRecordingRing accepts at most one cap, got %d", len(cap))
	}
	c := cap[0]
	if c == 0 {
		return fmt.Errorf("aimux: InitRecordingRing requires cap > 0 (got 0)")
	}
	rc := C.aimux_init_recording_ring(C.uint64_t(c))
	if rc != 0 {
		return fmt.Errorf("aimux: InitRecordingRing failed (rc=%d)", int32(rc))
	}
	return nil
}

// RecordingStop stops recording: the global recorder becomes None.
func RecordingStop() {
	C.aimux_recording_stop()
}

// RecordingFlush flushes the global recorder (blocks until JSONL is on disk;
// no-op for the ring recorder).
func RecordingFlush() {
	C.aimux_recording_flush()
}

// MockReplay creates a mock replay model from recorded JSONL (RFC-0023 P3):
// it returns recorded responses by input match — no real API is sent. The
// returned model works with GenerateText / StreamText.
func MockReplay(recordingsJsonl string) (*Model, error) {
	cJsonl := C.CString(recordingsJsonl)
	defer C.free(unsafe.Pointer(cJsonl))
	var cerr C.AimuxError
	C.aimux_error_clear(&cerr)
	h := C.aimux_mock_replay_new(cJsonl, &cerr)
	return wrapHandleU64(h, &cerr)
}

// RegisterProviders registers external OpenAI-compatible providers from a JSON
// config string (RFC-0020). Entries override same-named built-ins or add new
// ones. configJSON shape: { "providers": [ { "name", "base_url", ... } ] }.
func RegisterProviders(configJSON string) error {
	cJSON := C.CString(configJSON)
	defer C.free(unsafe.Pointer(cJSON))
	var cerr C.AimuxError
	C.aimux_error_clear(&cerr)
	rc := C.aimux_register_providers(cJSON, &cerr)
	if rc == 0 {
		return errorFromC(&cerr)
	}
	return nil
}

// Anthropic creates an Anthropic model instance, panicking on failure.
func Anthropic(apiKey, modelID string) *Model {
	return mustNew(NewAnthropic(apiKey, modelID))
}

// AnthropicWithBase creates an Anthropic model with a custom base URL, panicking on failure.
func AnthropicWithBase(apiKey, modelID, baseURL string) *Model {
	return mustNew(NewAnthropicWithBase(apiKey, modelID, baseURL))
}

func mustNew(m *Model, err error) *Model {
	if err != nil {
		panic(err)
	}
	return m
}

// wrapHandleU64 turns a constructor handle + optional AimuxError into *Model.
func wrapHandleU64(h C.uint64_t, cerr *C.AimuxError) (*Model, error) {
	if h == 0 {
		return nil, errorFromC(cerr)
	}
	m := &Model{handle: uint64(h)}
	runtime.SetFinalizer(m, func(m *Model) { m.Close() })
	return m, nil
}

func newModel(apiKey, modelID, baseURL, kind string) (*Model, error) {
	cKey := C.CString(apiKey)
	cModel := C.CString(modelID)
	defer C.free(unsafe.Pointer(cKey))
	defer C.free(unsafe.Pointer(cModel))

	var cerr C.AimuxError
	C.aimux_error_clear(&cerr)
	var h C.uint64_t
	if baseURL == "" {
		switch kind {
		case "anthropic":
			h = C.aimux_anthropic_new(cKey, cModel, &cerr)
		case "cohere":
			h = C.aimux_cohere_new(cKey, cModel, &cerr)
		case "mistral":
			h = C.aimux_mistral_new(cKey, cModel, &cerr)
		case "xai":
			h = C.aimux_xai_new(cKey, cModel, &cerr)
		default:
			h = C.aimux_openai_new(cKey, cModel, &cerr)
		}
	} else {
		cBase := C.CString(baseURL)
		defer C.free(unsafe.Pointer(cBase))
		switch kind {
		case "anthropic":
			h = C.aimux_anthropic_new_with_base(cKey, cModel, cBase, &cerr)
		case "cohere":
			h = C.aimux_cohere_new_with_base(cKey, cModel, cBase, &cerr)
		case "mistral":
			h = C.aimux_mistral_new_with_base(cKey, cModel, cBase, &cerr)
		case "xai":
			h = C.aimux_xai_new_with_base(cKey, cModel, cBase, &cerr)
		default:
			h = C.aimux_openai_new_with_base(cKey, cModel, cBase, &cerr)
		}
	}
	return wrapHandleU64(h, &cerr)
}

func newBedrockModel(accessKeyID, secretAccessKey, region, modelID, baseURL string) (*Model, error) {
	cAccess := C.CString(accessKeyID)
	cSecret := C.CString(secretAccessKey)
	cRegion := C.CString(region)
	cModel := C.CString(modelID)
	defer C.free(unsafe.Pointer(cAccess))
	defer C.free(unsafe.Pointer(cSecret))
	defer C.free(unsafe.Pointer(cRegion))
	defer C.free(unsafe.Pointer(cModel))

	var cerr C.AimuxError
	C.aimux_error_clear(&cerr)
	var h C.uint64_t
	if baseURL == "" {
		h = C.aimux_bedrock_new(cAccess, cSecret, cRegion, cModel, &cerr)
	} else {
		cBase := C.CString(baseURL)
		defer C.free(unsafe.Pointer(cBase))
		h = C.aimux_bedrock_new_with_base(cAccess, cSecret, cRegion, cModel, cBase, &cerr)
	}
	return wrapHandleU64(h, &cerr)
}

func newVertexModel(accessToken, project, location, modelID, baseURL string) (*Model, error) {
	cToken := C.CString(accessToken)
	cProject := C.CString(project)
	cLocation := C.CString(location)
	cModel := C.CString(modelID)
	defer C.free(unsafe.Pointer(cToken))
	defer C.free(unsafe.Pointer(cProject))
	defer C.free(unsafe.Pointer(cLocation))
	defer C.free(unsafe.Pointer(cModel))

	var cerr C.AimuxError
	C.aimux_error_clear(&cerr)
	var h C.uint64_t
	if baseURL == "" {
		h = C.aimux_vertex_new(cToken, cProject, cLocation, cModel, &cerr)
	} else {
		cBase := C.CString(baseURL)
		defer C.free(unsafe.Pointer(cBase))
		h = C.aimux_vertex_new_with_base(cToken, cProject, cLocation, cModel, cBase, &cerr)
	}
	return wrapHandleU64(h, &cerr)
}

func newAnthropicAwsModel(apiKey, region, modelID, baseURL string) (*Model, error) {
	cKey := C.CString(apiKey)
	cRegion := C.CString(region)
	cModel := C.CString(modelID)
	defer C.free(unsafe.Pointer(cKey))
	defer C.free(unsafe.Pointer(cRegion))
	defer C.free(unsafe.Pointer(cModel))

	var cerr C.AimuxError
	C.aimux_error_clear(&cerr)
	var h C.uint64_t
	if baseURL == "" {
		h = C.aimux_anthropic_aws_new(cKey, cRegion, cModel, &cerr)
	} else {
		cBase := C.CString(baseURL)
		defer C.free(unsafe.Pointer(cBase))
		h = C.aimux_anthropic_aws_new_with_base(cKey, cRegion, cModel, cBase, &cerr)
	}
	return wrapHandleU64(h, &cerr)
}

func newAzureModel(apiKey, resourceOrBase, deployment, apiVersion string, useBase bool) (*Model, error) {
	cKey := C.CString(apiKey)
	cResource := C.CString(resourceOrBase)
	cDeployment := C.CString(deployment)
	cVersion := C.CString(apiVersion)
	defer C.free(unsafe.Pointer(cKey))
	defer C.free(unsafe.Pointer(cResource))
	defer C.free(unsafe.Pointer(cDeployment))
	defer C.free(unsafe.Pointer(cVersion))

	var cerr C.AimuxError
	C.aimux_error_clear(&cerr)
	var h C.uint64_t
	if useBase {
		h = C.aimux_azure_new_with_base(cKey, cResource, cDeployment, cVersion, &cerr)
	} else {
		h = C.aimux_azure_new(cKey, cResource, cDeployment, cVersion, &cerr)
	}
	return wrapHandleU64(h, &cerr)
}

// Provider creates a model from the built-in registry by provider name
// (RFC-0017 phase 4). apiKey may be "" to read the provider's env var.
//
//	m, err := aimux.Provider("groq", "", "llama-3.3-70b")
func Provider(name, apiKey, modelID string) (*Model, error) {
	return ProviderWithBase(name, apiKey, modelID, "")
}

// ProviderWithBase is Provider with a base URL override (config_json).
func ProviderWithBase(name, apiKey, modelID, baseURL string) (*Model, error) {
	if baseURL == "" {
		return ProviderWithConfig(name, apiKey, modelID, nil)
	}
	return ProviderWithConfig(name, apiKey, modelID, &ProviderConfig{BaseURL: baseURL})
}

// ProviderConfig mirrors the Rust ProviderOptions accepted by
// aimux_provider_new's config_json (RFC-0017 §3.4). Zero-value fields are
// omitted; MaxRetries is a pointer so 0 (disable retries) is expressible.
type ProviderConfig struct {
	BaseURL       string            `json:"base_url,omitempty"`
	Headers       map[string]string `json:"headers,omitempty"`
	Organization  string            `json:"organization,omitempty"`
	Project       string            `json:"project,omitempty"`
	MaxRetries    *uint32           `json:"max_retries,omitempty"`
	BodyOverrides map[string]any    `json:"body_overrides,omitempty"`
}

// ProviderWithConfig is Provider with the full ProviderOptions config.
// cfg may be nil for defaults.
func ProviderWithConfig(name, apiKey, modelID string, cfg *ProviderConfig) (*Model, error) {
	cName := C.CString(name)
	cModel := C.CString(modelID)
	defer C.free(unsafe.Pointer(cName))
	defer C.free(unsafe.Pointer(cModel))

	var cKey *C.char
	if apiKey != "" {
		cKey = C.CString(apiKey)
		defer C.free(unsafe.Pointer(cKey))
	}

	var cConfig *C.char
	if cfg != nil {
		buf, err := json.Marshal(cfg)
		if err != nil {
			return nil, fmt.Errorf("aimux: marshal provider config: %w", err)
		}
		cConfig = C.CString(string(buf))
		defer C.free(unsafe.Pointer(cConfig))
	}

	var cerr C.AimuxError
	C.aimux_error_clear(&cerr)
	h := C.aimux_provider_new(cName, cKey, cModel, cConfig, &cerr)
	return wrapHandleU64(h, &cerr)
}

// ── Provider handles (RFC-0027) ─────────────────────────────────────────────

// ProviderHandle is a provider handle created by CreateProvider. It supports
// ListModels (runtime discovery) and Model (build a model from a discovered id).
type ProviderHandle struct {
	mu     sync.RWMutex
	handle uint64
	closed bool
}

// Close releases the native handle. Safe to call multiple times.
func (p *ProviderHandle) Close() error {
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.closed {
		return nil
	}
	p.closed = true
	if p.handle != 0 {
		C.aimux_drop_handle(C.uint64_t(p.handle))
		p.handle = 0
	}
	runtime.SetFinalizer(p, nil)
	return nil
}

// ListModels lists models available on this provider (runtime discovery via
// the provider's /models endpoint), enriched with community knowledge (anya2a)
// when available. Returns a JSON array of RuntimeModel.
func (p *ProviderHandle) ListModels() (string, error) {
	p.mu.RLock()
	defer p.mu.RUnlock()
	if p.closed || p.handle == 0 {
		return "", newError(CodeInvalidArgument, "aimux: provider handle is closed")
	}
	return ffiString(func(cerr *C.AimuxError) *C.char {
		return C.aimux_provider_list_models(C.uint64_t(p.handle), cerr)
	})
}

// Model builds a language model from a discovered model id.
func (p *ProviderHandle) Model(modelID string) (*Model, error) {
	p.mu.RLock()
	defer p.mu.RUnlock()
	if p.closed || p.handle == 0 {
		return nil, newError(CodeInvalidArgument, "aimux: provider handle is closed")
	}
	cModel := C.CString(modelID)
	defer C.free(unsafe.Pointer(cModel))
	var cerr C.AimuxError
	C.aimux_error_clear(&cerr)
	h := C.aimux_provider_model(C.uint64_t(p.handle), cModel, &cerr)
	return wrapHandleU64(h, &cerr)
}

// CreateProvider creates a provider handle for a registry-backed provider.
// Unlike Provider (which binds to a single modelID), this returns a handle
// that supports ListModels() and Model().
func CreateProvider(name, apiKey string, cfg *ProviderConfig) (*ProviderHandle, error) {
	cName := C.CString(name)
	defer C.free(unsafe.Pointer(cName))

	var cKey *C.char
	if apiKey != "" {
		cKey = C.CString(apiKey)
		defer C.free(unsafe.Pointer(cKey))
	}

	var cConfig *C.char
	if cfg != nil {
		buf, err := json.Marshal(cfg)
		if err != nil {
			return nil, fmt.Errorf("aimux: marshal provider config: %w", err)
		}
		cConfig = C.CString(string(buf))
		defer C.free(unsafe.Pointer(cConfig))
	}

	var cerr C.AimuxError
	C.aimux_error_clear(&cerr)
	h := C.aimux_provider_handle_new(cName, cKey, cConfig, &cerr)
	if h == 0 {
		return nil, errorFromC(&cerr)
	}
	p := &ProviderHandle{handle: uint64(h)}
	runtime.SetFinalizer(p, func(p *ProviderHandle) { p.Close() })
	return p, nil
}

// ── Model specs (RFC-0027) ──────────────────────────────────────────────────

// GetModelSpecs fetches the community model catalogue (anya2a). Returns a JSON
// string representing the Catalogue (provider → model_id → ModelSpec).
// Thin fetch — no caching. sourceURL may be "" for the default endpoint.
func GetModelSpecs(sourceURL string) (string, error) {
	var cURL *C.char
	if sourceURL != "" {
		cURL = C.CString(sourceURL)
		defer C.free(unsafe.Pointer(cURL))
	}
	return ffiString(func(cerr *C.AimuxError) *C.char {
		return C.aimux_get_model_specs(cURL, cerr)
	})
}

// ── Non-streaming generation ────────────────────────────────────────────────

// GenerateText performs non-streaming text generation.
//
// Parameters:
//   - promptJson: JSON prompt (bare value like "\"text\"" or "[{...}] messages array",
//     or {"prompt": <value>}).
//   - optsJson:   JSON-serialized GenerateTextOptions ("" or null for defaults).
//
// Returns the JSON-serialized GenerateTextResult, or an error if the FFI call failed.
//
//	result, err := model.GenerateText(`"What is Rust?"`, "")
func (m *Model) GenerateText(promptJson, optsJson string) (string, error) {
	handle, release, err := m.acquireHandle()
	if err != nil {
		return "", err
	}
	defer release()

	cPrompt := C.CString(promptJson)
	defer C.free(unsafe.Pointer(cPrompt))

	var cOpts *C.char
	if optsJson != "" {
		cOpts = C.CString(optsJson)
		defer C.free(unsafe.Pointer(cOpts))
	}

	return ffiString(func(cerr *C.AimuxError) *C.char {
		return C.aimux_generate_text(C.uint64_t(handle), cPrompt, cOpts, cerr)
	})
}

// GenerateTextAsOpenAI performs non-streaming text generation and returns the
// result as an OpenAI Chat Completion JSON string (RFC-0026).
//
// Same as GenerateText, but the returned JSON is a serialized ChatCompletion
// (OpenAI "chat.completion" object) rather than a GenerateTextResult. Works
// with any provider.
func (m *Model) GenerateTextAsOpenAI(promptJson, optsJson string) (string, error) {
	handle, release, err := m.acquireHandle()
	if err != nil {
		return "", err
	}
	defer release()

	cPrompt := C.CString(promptJson)
	defer C.free(unsafe.Pointer(cPrompt))

	var cOpts *C.char
	if optsJson != "" {
		cOpts = C.CString(optsJson)
		defer C.free(unsafe.Pointer(cOpts))
	}

	return ffiString(func(cerr *C.AimuxError) *C.char {
		return C.aimux_generate_text_as_openai(C.uint64_t(handle), cPrompt, cOpts, cerr)
	})
}

// ── Streaming generation ─────────────────────────────────────────────────────

// streamEntry holds the channel and error for an active stream.
type streamEntry struct {
	parts        chan string
	mu           sync.Mutex
	err          error
	closeOnce    sync.Once
	terminalOnce sync.Once
	terminal     chan struct{}
	cancelled    chan struct{}
	abortHandle  uint64
}

// streamRegistry maps stream IDs to active stream entries. This avoids passing
// Go pointers into C (cgo pointer rules forbid passing Go memory containing Go
// pointers like channels). The ID is a plain int64_t.
var (
	streamRegMu  sync.Mutex
	streamReg    = make(map[int64]*streamEntry)
	streamNextID int64
)

func registerStream(abortHandle uint64) (*streamEntry, int64) {
	streamRegMu.Lock()
	defer streamRegMu.Unlock()
	streamNextID++
	e := &streamEntry{
		parts:       make(chan string, 256),
		terminal:    make(chan struct{}),
		cancelled:   make(chan struct{}),
		abortHandle: abortHandle,
	}
	streamReg[streamNextID] = e
	return e, streamNextID
}

func lookupStream(id int64) *streamEntry {
	streamRegMu.Lock()
	defer streamRegMu.Unlock()
	return streamReg[id]
}

func unregisterStream(id int64) {
	streamRegMu.Lock()
	defer streamRegMu.Unlock()
	delete(streamReg, id)
}

// closeParts safely closes the stream's parts channel exactly once, guarding
// against the engine firing both on_done and on_error for the same stream
// (which would otherwise panic on double close).
func (e *streamEntry) closeParts() {
	e.closeOnce.Do(func() { close(e.parts) })
}

// markTerminal records the first terminal result. A cancellation also wakes
// callbacks that are waiting to send to a full parts channel.
func (e *streamEntry) markTerminal(err error, cancelled bool) bool {
	marked := false
	e.terminalOnce.Do(func() {
		marked = true
		e.mu.Lock()
		e.err = err
		e.mu.Unlock()
		if cancelled {
			close(e.cancelled)
		}
		close(e.terminal)
	})
	return marked
}

func (e *streamEntry) cancel(err error) {
	if err == nil {
		err = context.Canceled
	}
	if e.markTerminal(err, true) && e.abortHandle != 0 {
		C.aimux_abort_signal_abort(C.uint64_t(e.abortHandle))
	}
}

// Stream is a handle to an in-progress or completed stream.
// Consume parts via the Parts() channel; check Err() after the channel closes.
//
// The channel is buffered (256). Call Cancel when the caller stops consuming.
type Stream struct {
	parts <-chan string
	entry *streamEntry
}

// Parts returns a receive-only channel of StreamPart JSON strings.
// The channel is closed when the stream ends (normally or on error).
func (s *Stream) Parts() <-chan string { return s.parts }

// Err returns any error that occurred during streaming.
// Call this after Parts() channel closes.
func (s *Stream) Err() error {
	s.entry.mu.Lock()
	defer s.entry.mu.Unlock()
	return s.entry.err
}

// Cancel stops this stream. It is safe to call more than once.
func (s *Stream) Cancel() {
	s.entry.cancel(context.Canceled)
}

// StreamText performs streaming text generation.
//
// It returns immediately with a *Stream. Consume parts via stream.Parts():
//
//	stream := model.StreamText(`"Write a haiku"`, "")
//	for part := range stream.Parts() {
//	    fmt.Println(part) // StreamPart JSON
//	}
//	if err := stream.Err(); err != nil {
//	    log.Fatal(err)
//	}
//
// Drain the Parts channel or call Cancel when you stop reading. Use
// StreamTextContext to connect cancellation to a context.
func (m *Model) StreamText(promptJson, optsJson string) *Stream {
	return m.StreamTextContext(context.Background(), promptJson, optsJson)
}

// StreamTextContext performs streaming text generation. Context cancellation
// stops the native request and makes Err return the context error.
func (m *Model) StreamTextContext(ctx context.Context, promptJson, optsJson string) *Stream {
	if ctx == nil {
		ctx = context.Background()
	}
	abortHandle := uint64(C.aimux_abort_signal_new())
	entry, id := registerStream(abortHandle)

	if err := ctx.Err(); err != nil {
		entry.cancel(err)
	} else if done := ctx.Done(); done != nil {
		go func() {
			select {
			case <-done:
				entry.cancel(ctx.Err())
			case <-entry.terminal:
			}
		}()
	}

	go func() {
		defer unregisterStream(id)
		defer C.aimux_abort_signal_drop(C.uint64_t(abortHandle))
		// Safety net: ensure the channel is always closed even if the
		// native layer never fires on_done/on_error (defensive against
		// future bugs or panic edges in the FFI layer).
		defer func() {
			entry.markTerminal(nil, false)
			entry.closeParts()
		}()

		handle, release, err := m.acquireHandle()
		if err != nil {
			entry.markTerminal(err, false)
			return
		}
		defer release()

		cPrompt := C.CString(promptJson)
		defer C.free(unsafe.Pointer(cPrompt))

		var cOpts *C.char
		if optsJson != "" {
			cOpts = C.CString(optsJson)
			defer C.free(unsafe.Pointer(cOpts))
		}

		var cerr C.AimuxError
		C.aimux_error_clear(&cerr)
		rc := C.do_stream(
			C.uint64_t(handle),
			C.uint64_t(abortHandle),
			cPrompt,
			cOpts,
			C.int64_t(id),
			&cerr,
		)
		if rc == 0 {
			entry.markTerminal(errorFromC(&cerr), false)
		}
	}()

	return &Stream{parts: entry.parts, entry: entry}
}

// StreamTextAsOpenAI performs streaming text generation with OpenAI Chat
// Completion output (RFC-0026).
//
// Same as StreamText, but each part in the Parts() channel is a serialized
// ChatCompletionChunk (OpenAI "chat.completion.chunk" object). Works with any
// provider.
//
// Opts may carry providerOptions.openai.stream_options with include_usage
// (bool, default true) and include_reasoning (bool, default true).
func (m *Model) StreamTextAsOpenAI(promptJson, optsJson string) *Stream {
	return m.StreamTextAsOpenAIContext(context.Background(), promptJson, optsJson)
}

// StreamTextAsOpenAIContext performs streaming OpenAI-compatible generation.
// Context cancellation stops the native request.
func (m *Model) StreamTextAsOpenAIContext(ctx context.Context, promptJson, optsJson string) *Stream {
	if ctx == nil {
		ctx = context.Background()
	}
	abortHandle := uint64(C.aimux_abort_signal_new())
	entry, id := registerStream(abortHandle)

	if err := ctx.Err(); err != nil {
		entry.cancel(err)
	} else if done := ctx.Done(); done != nil {
		go func() {
			select {
			case <-done:
				entry.cancel(ctx.Err())
			case <-entry.terminal:
			}
		}()
	}

	go func() {
		defer unregisterStream(id)
		defer C.aimux_abort_signal_drop(C.uint64_t(abortHandle))
		// Safety net: ensure the channel is always closed even if the
		// native layer never fires on_done/on_error.
		defer func() {
			entry.markTerminal(nil, false)
			entry.closeParts()
		}()

		handle, release, err := m.acquireHandle()
		if err != nil {
			entry.markTerminal(err, false)
			return
		}
		defer release()

		cPrompt := C.CString(promptJson)
		defer C.free(unsafe.Pointer(cPrompt))

		var cOpts *C.char
		if optsJson != "" {
			cOpts = C.CString(optsJson)
			defer C.free(unsafe.Pointer(cOpts))
		}

		var cerr C.AimuxError
		C.aimux_error_clear(&cerr)
		rc := C.do_stream_openai(
			C.uint64_t(handle),
			C.uint64_t(abortHandle),
			cPrompt,
			cOpts,
			C.int64_t(id),
			&cerr,
		)
		if rc == 0 {
			entry.markTerminal(errorFromC(&cerr), false)
		}
	}()

	return &Stream{parts: entry.parts, entry: entry}
}

// errorFromC maps aimux-ffi AimuxError into *Error (openai-go style).
// It takes ownership of e.message and e.error_value (allocated by the callee
// on failure) and frees them via aimux_free_string.
func errorFromC(e *C.AimuxError) error {
	if e == nil || e.code == C.AIMUX_OK {
		return newError(CodeUnknown, "aimux: operation failed")
	}
	var msg string
	if e.message != nil {
		msg = C.GoString(e.message)
		C.aimux_free_string(e.message)
		e.message = nil
	}
	if msg == "" {
		msg = fmt.Sprintf("aimux: %s", Code(e.code).String())
	}
	var errValue string
	if e.error_value != nil {
		errValue = C.GoString(e.error_value)
		C.aimux_free_string(e.error_value)
		e.error_value = nil
	}
	// Status defaults for well-known HTTP kinds mirror Kotlin/Flutter/Node
	// when C reports -1 (no concrete HTTP status): RateLimited→429,
	// Auth/TokenExpired→401, ModelNotFound→404. See bindings/kotlin
	// Errors.kt createByCode and bindings/flutter errors.dart fromC.
	status := defaultStatus(Code(e.code), int(e.status))
	return &Error{
		Code:       Code(e.code),
		Message:    msg,
		Status:     status,
		RetryMs:    int64(e.retry_ms),
		ErrorValue: errValue,
	}
}

// ffiString runs an FFI call returning an aimux-allocated C string, mapping
// the NULL-sentinel failure path through errorFromC. It clears *cerr up front
// (cheap at 24 bytes) as defense against callees that fail without writing it.
func ffiString(call func(cerr *C.AimuxError) *C.char) (string, error) {
	var cerr C.AimuxError
	C.aimux_error_clear(&cerr)
	ptr := call(&cerr)
	if ptr == nil {
		return "", errorFromC(&cerr)
	}
	defer C.aimux_free_string(ptr)
	return C.GoString(ptr), nil
}

// ── C→Go callback trampolines (called by trampoline_part/done) ─────────

//export goStreamPart
func goStreamPart(id C.int64_t, json *C.char) {
	e := lookupStream(int64(id))
	if e == nil {
		return
	}
	if json == nil {
		return
	}
	select {
	case e.parts <- C.GoString(json):
	case <-e.cancelled:
	}
}

//export goStreamDone
func goStreamDone(id C.int64_t) {
	e := lookupStream(int64(id))
	if e == nil {
		return
	}
	e.markTerminal(nil, false)
	e.closeParts()
}

// Ensure io.Closer interface is satisfied.
var _ io.Closer = (*Model)(nil)
