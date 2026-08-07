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

// ── Thread-local stream context ID ─────────────────────────────────────────
//
// aimux_stream_text uses push callbacks (RFC-0001 §4.1) that carry no user-data
// pointer. We bridge them to Go via a thread-local integer ID that indexes into
// a Go-side registry (streamRegistry). The C function do_stream sets the ID
// before calling aimux_stream_text and clears it after — all on the same OS
// thread, so __thread is safe without runtime.LockOSThread.

static __thread int64_t current_stream_id = 0;

// Go-side callback trampolines (forwarded via //export below).
extern void goStreamPart(int64_t id, char* json);
extern void goStreamDone(int64_t id);
extern void goStreamError(int64_t id, char* err);

static void trampoline_part(const char* json) {
    if (current_stream_id) goStreamPart(current_stream_id, (char*)json);
}
static void trampoline_done(void) {
    if (current_stream_id) goStreamDone(current_stream_id);
}
static void trampoline_error(const char* err) {
    if (current_stream_id) goStreamError(current_stream_id, (char*)err);
}

// do_stream: set the thread-local ID, call the blocking cancelable stream
// function, then clear the ID. The callbacks fire during this call.
static void do_stream(uint64_t handle, uint64_t abort_handle,
                      const char* prompt, const char* opts, int64_t id) {
    current_stream_id = id;
    aimux_stream_text_with_abort(handle, abort_handle, prompt, opts,
                                 trampoline_part, trampoline_done,
                                 trampoline_error);
    current_stream_id = 0;
}

// do_stream_openai: same as do_stream but emits OpenAI Chat Completion chunks
// (RFC-0026). Reuses the same trampolines — they just forward JSON strings.
static void do_stream_openai(uint64_t handle, uint64_t abort_handle,
                             const char* prompt, const char* opts, int64_t id) {
    current_stream_id = id;
    aimux_stream_text_as_openai_with_abort(handle, abort_handle, prompt, opts,
                                           trampoline_part, trampoline_done,
                                           trampoline_error);
    current_stream_id = 0;
}

// ── Multimodal constructors and operations (not in aimux-ffi.h yet, but
//    exported as C symbols from libaimux_ffi.a) ────────────────────────────

// Embedding
char *aimux_openai_embedding_new(const char *api_key, const char *model_id);
char *aimux_openai_embedding_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_cohere_embedding_new(const char *api_key, const char *model_id);
char *aimux_cohere_embedding_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_google_embedding_new(const char *api_key, const char *model_id);
char *aimux_google_embedding_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_embed(uint64_t handle, const char *values_json, const char *opts_json);

// Registry provider (RFC-0017 phase 4): name + optional api_key/env + config JSON
char *aimux_provider_new(const char *name, const char *api_key, const char *model_id, const char *config_json);
char *aimux_provider_from_env(const char *name, const char *model_id);

// Speech (TTS)
char *aimux_openai_speech_new(const char *api_key, const char *model_id);
char *aimux_openai_speech_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_speech_generate(uint64_t handle, const char *opts_json);

// Image
char *aimux_openai_image_new(const char *api_key, const char *model_id);
char *aimux_openai_image_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_google_image_new(const char *api_key, const char *model_id);
char *aimux_google_image_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_image_generate(uint64_t handle, const char *opts_json);

// Transcription (STT)
char *aimux_openai_transcription_new(const char *api_key, const char *model_id);
char *aimux_openai_transcription_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_transcription_generate(uint64_t handle, const char *audio_base64, const char *media_type, const char *opts_json);

// Files
char *aimux_openai_files_new(const char *api_key);
char *aimux_openai_files_new_with_base(const char *api_key, const char *base_url);
char *aimux_file_upload(uint64_t handle, const char *data_base64, const char *media_type, const char *opts_json);

// Reranking
char *aimux_cohere_reranking_new(const char *api_key, const char *model_id);
char *aimux_cohere_reranking_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_rerank(uint64_t handle, const char *opts_json);

// Video
char *aimux_google_video_new(const char *api_key, const char *model_id);
char *aimux_google_video_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_video_generate(uint64_t handle, const char *opts_json);

// Search
char *aimux_tavily_search_new(const char *api_key, const char *model_id);
char *aimux_tavily_search_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_search(uint64_t handle, const char *opts_json);
*/
import "C"

import (
	"context"
	"encoding/json"
	"errors"
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
		return 0, nil, errors.New("aimux: model already closed")
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
// ring with the given capacity. cap == 0 falls back to the default (2048).
func InitRecordingRing(cap uint64) {
	if cap == 0 {
		cap = 2048
	}
	C.aimux_init_recording_ring(C.uint64_t(cap))
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
	ptr := C.aimux_mock_replay_new(cJsonl)
	handle, err := parseHandleJSON(ptr)
	if err != nil {
		return nil, err
	}
	return &Model{handle: handle}, nil
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

// parseHandleJSON parses a constructor's JSON result
// (`{"handle":<u64>}` on success, `{"error":...}` on failure), freeing the
// C string. Mirrors extractError, which does the same for GenerateText.
func parseHandleJSON(ptr *C.char) (uint64, error) {
	if ptr == nil {
		return 0, errors.New("aimux: constructor returned null")
	}
	defer C.aimux_free_string(ptr)
	result := C.GoString(ptr)
	if msg := extractError(result); msg != "" {
		return 0, fmt.Errorf("aimux: %s", msg)
	}
	var envelope struct {
		Handle uint64 `json:"handle"`
	}
	if err := json.Unmarshal([]byte(result), &envelope); err != nil {
		return 0, fmt.Errorf("aimux: failed to parse constructor result: %w", err)
	}
	if envelope.Handle == 0 {
		return 0, fmt.Errorf("aimux: constructor returned neither handle nor error: %s", result)
	}
	return envelope.Handle, nil
}

func newModel(apiKey, modelID, baseURL, kind string) (*Model, error) {
	m := &Model{}
	cKey := C.CString(apiKey)
	cModel := C.CString(modelID)
	defer C.free(unsafe.Pointer(cKey))
	defer C.free(unsafe.Pointer(cModel))

	var ptr *C.char
	if baseURL == "" {
		switch kind {
		case "anthropic":
			ptr = C.aimux_anthropic_new(cKey, cModel)
		case "cohere":
			ptr = C.aimux_cohere_new(cKey, cModel)
		case "mistral":
			ptr = C.aimux_mistral_new(cKey, cModel)
		case "xai":
			ptr = C.aimux_xai_new(cKey, cModel)
		default:
			ptr = C.aimux_openai_new(cKey, cModel)
		}
	} else {
		cBase := C.CString(baseURL)
		defer C.free(unsafe.Pointer(cBase))
		switch kind {
		case "anthropic":
			ptr = C.aimux_anthropic_new_with_base(cKey, cModel, cBase)
		case "cohere":
			ptr = C.aimux_cohere_new_with_base(cKey, cModel, cBase)
		case "mistral":
			ptr = C.aimux_mistral_new_with_base(cKey, cModel, cBase)
		case "xai":
			ptr = C.aimux_xai_new_with_base(cKey, cModel, cBase)
		default:
			ptr = C.aimux_openai_new_with_base(cKey, cModel, cBase)
		}
	}
	return wrapHandle(m, ptr)
}

func newBedrockModel(accessKeyID, secretAccessKey, region, modelID, baseURL string) (*Model, error) {
	m := &Model{}
	cAccess := C.CString(accessKeyID)
	cSecret := C.CString(secretAccessKey)
	cRegion := C.CString(region)
	cModel := C.CString(modelID)
	defer C.free(unsafe.Pointer(cAccess))
	defer C.free(unsafe.Pointer(cSecret))
	defer C.free(unsafe.Pointer(cRegion))
	defer C.free(unsafe.Pointer(cModel))

	var ptr *C.char
	if baseURL == "" {
		ptr = C.aimux_bedrock_new(cAccess, cSecret, cRegion, cModel)
	} else {
		cBase := C.CString(baseURL)
		defer C.free(unsafe.Pointer(cBase))
		ptr = C.aimux_bedrock_new_with_base(cAccess, cSecret, cRegion, cModel, cBase)
	}
	return wrapHandle(m, ptr)
}

func newVertexModel(accessToken, project, location, modelID, baseURL string) (*Model, error) {
	m := &Model{}
	cToken := C.CString(accessToken)
	cProject := C.CString(project)
	cLocation := C.CString(location)
	cModel := C.CString(modelID)
	defer C.free(unsafe.Pointer(cToken))
	defer C.free(unsafe.Pointer(cProject))
	defer C.free(unsafe.Pointer(cLocation))
	defer C.free(unsafe.Pointer(cModel))

	var ptr *C.char
	if baseURL == "" {
		ptr = C.aimux_vertex_new(cToken, cProject, cLocation, cModel)
	} else {
		cBase := C.CString(baseURL)
		defer C.free(unsafe.Pointer(cBase))
		ptr = C.aimux_vertex_new_with_base(cToken, cProject, cLocation, cModel, cBase)
	}
	return wrapHandle(m, ptr)
}

func newAnthropicAwsModel(apiKey, region, modelID, baseURL string) (*Model, error) {
	m := &Model{}
	cKey := C.CString(apiKey)
	cRegion := C.CString(region)
	cModel := C.CString(modelID)
	defer C.free(unsafe.Pointer(cKey))
	defer C.free(unsafe.Pointer(cRegion))
	defer C.free(unsafe.Pointer(cModel))

	var ptr *C.char
	if baseURL == "" {
		ptr = C.aimux_anthropic_aws_new(cKey, cRegion, cModel)
	} else {
		cBase := C.CString(baseURL)
		defer C.free(unsafe.Pointer(cBase))
		ptr = C.aimux_anthropic_aws_new_with_base(cKey, cRegion, cModel, cBase)
	}
	return wrapHandle(m, ptr)
}

func newAzureModel(apiKey, resourceOrBase, deployment, apiVersion string, useBase bool) (*Model, error) {
	m := &Model{}
	cKey := C.CString(apiKey)
	cResource := C.CString(resourceOrBase)
	cDeployment := C.CString(deployment)
	cVersion := C.CString(apiVersion)
	defer C.free(unsafe.Pointer(cKey))
	defer C.free(unsafe.Pointer(cResource))
	defer C.free(unsafe.Pointer(cDeployment))
	defer C.free(unsafe.Pointer(cVersion))

	var ptr *C.char
	if useBase {
		ptr = C.aimux_azure_new_with_base(cKey, cResource, cDeployment, cVersion)
	} else {
		ptr = C.aimux_azure_new(cKey, cResource, cDeployment, cVersion)
	}
	return wrapHandle(m, ptr)
}

// wrapHandle parses a constructor's JSON result into m, with a finalizer as
// a safety net (callers should still use Close).
func wrapHandle(m *Model, ptr *C.char) (*Model, error) {
	h, err := parseHandleJSON(ptr)
	if err != nil {
		return nil, err
	}
	m.handle = h
	runtime.SetFinalizer(m, func(m *Model) { m.Close() })
	return m, nil
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
	m := &Model{}
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

	ptr := C.aimux_provider_new(cName, cKey, cModel, cConfig)
	return wrapHandle(m, ptr)
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

	ptr := C.aimux_generate_text(C.uint64_t(handle), cPrompt, cOpts)
	if ptr == nil {
		return "", errors.New("aimux: generate_text returned null")
	}
	defer C.aimux_free_string(ptr)

	result := C.GoString(ptr)
	if msg := extractError(result); msg != "" {
		return "", fmt.Errorf("aimux: %s", msg)
	}
	return result, nil
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

	ptr := C.aimux_generate_text_as_openai(C.uint64_t(handle), cPrompt, cOpts)
	if ptr == nil {
		return "", errors.New("aimux: generate_text_as_openai returned null")
	}
	defer C.aimux_free_string(ptr)

	result := C.GoString(ptr)
	if msg := extractError(result); msg != "" {
		return "", fmt.Errorf("aimux: %s", msg)
	}
	return result, nil
}

// extractError checks if the JSON result is an error envelope
// ({"error":"..."}) and returns the error message, or "" if not an error.
func extractError(result string) string {
	var envelope struct {
		Error *string `json:"error"`
	}
	if err := json.Unmarshal([]byte(result), &envelope); err != nil {
		return "" // not valid JSON — let the caller handle it
	}
	if envelope.Error != nil {
		return *envelope.Error
	}
	return ""
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
		runtime.LockOSThread()
		defer runtime.UnlockOSThread()
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

		C.do_stream(
			C.uint64_t(handle),
			C.uint64_t(abortHandle),
			cPrompt,
			cOpts,
			C.int64_t(id),
		)
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
		runtime.LockOSThread()
		defer runtime.UnlockOSThread()
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

		C.do_stream_openai(
			C.uint64_t(handle),
			C.uint64_t(abortHandle),
			cPrompt,
			cOpts,
			C.int64_t(id),
		)
	}()

	return &Stream{parts: entry.parts, entry: entry}
}

// ── C→Go callback trampolines (called by trampoline_part/done/error) ─────────

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

//export goStreamError
func goStreamError(id C.int64_t, err *C.char) {
	e := lookupStream(int64(id))
	if e == nil {
		return
	}
	msg := "unknown stream error"
	if err != nil {
		msg = C.GoString(err)
		// Extract the error message from the JSON envelope if present.
		if extracted := extractError(msg); extracted != "" {
			msg = extracted
		}
	}
	e.markTerminal(errors.New(msg), false)
	e.closeParts()
}

// Ensure io.Closer interface is satisfied.
var _ io.Closer = (*Model)(nil)
