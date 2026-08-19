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
extern void goStreamPart(uintptr_t id, char* json);

static void trampoline_part(const char* json, void* stream_ctx) {
    uintptr_t id = (uintptr_t)stream_ctx;
    if (id) goStreamPart(id, (char*)json);
}
static void trampoline_done(void* stream_ctx) {
    (void)stream_ctx;
}

// do_stream: blocking cancelable stream; id is passed as stream_ctx.
// NULL return after on_done = clean end; non-NULL = failure (no on_done).
static aimux_error_t* do_stream(uint64_t handle, uint64_t abort_handle,
                                    const char* prompt, const char* opts, uintptr_t id) {
    return aimux_stream_text_with_abort(
        handle, abort_handle, prompt, opts,
        trampoline_part, trampoline_done,
        (void*)id);
}

static aimux_error_t* do_stream_openai(uint64_t handle, uint64_t abort_handle,
                                           const char* prompt, const char* opts, uintptr_t id) {
    return aimux_stream_text_as_openai_with_abort(
        handle, abort_handle, prompt, opts,
        trampoline_part, trampoline_done,
        (void*)id);
}

*/
import "C"

import (
	"context"
	"errors"
	"fmt"
	"io"
	"runtime"
	"runtime/cgo"
	"strings"
	"sync/atomic"
	"unsafe"
)

// ── Model handle wrapper ───────────────────────────────────────────────────

// Model is a model instance backed by a Rust Arc<dyn LanguageModel>.
//
// It implements io.Closer — you MUST call Close (or use defer) to release the
// native handle and avoid memory leaks.
//
// Model must not be copied after first use. Its methods and Close are safe to
// call concurrently. Close prevents future calls but does not wait for calls
// that have already entered the Rust registry; those calls own a cloned Arc
// and may finish normally.
type Model struct {
	id atomic.Uint64 // 0 means closed
	// traced is true only for handles produced by Trace/TraceAudited, the
	// only ones with a trace store behind them. Set at construction and never
	// mutated, so the trace guard reads it without synchronization.
	traced bool
}

// Close releases the native handle. Safe to call multiple times.
// It never waits for in-flight model calls or streams.
func (m *Model) Close() error {
	if m == nil {
		return nil
	}
	if id := m.id.Swap(0); id != 0 {
		C.aimux_drop_handle(C.uint64_t(id))
		runtime.SetFinalizer(m, nil)
	}
	return nil
}

// handle returns a snapshot of the native handle. Callers must keep m alive
// until after the C call (runtime.KeepAlive) so its finalizer cannot drop the
// registry entry between this load and Rust's Arc clone.
func (m *Model) handle() (uint64, error) {
	if m != nil {
		if id := m.id.Load(); id != 0 {
			return id, nil
		}
	}
	return 0, fmt.Errorf("%w: model", ErrClosed)
}

// modelHandles snapshots models in caller order without taking Go locks.
// A concurrent Close races only with Rust's registry lookup: either the call
// clones the Arc or it receives an invalid-handle error, never a use-after-free.
func modelHandles(models []*Model) ([]uint64, error) {
	out := make([]uint64, len(models))
	for i, m := range models {
		if m == nil {
			return nil, fmt.Errorf("model[%d] is nil", i)
		}
		id, err := m.handle()
		if err != nil {
			return nil, err
		}
		out[i] = id
	}
	return out, nil
}

func keepModelsAlive(models []*Model) {
	for _, m := range models {
		runtime.KeepAlive(m)
	}
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

// OpenAI creates an OpenAI model instance. Must-style (regexp.MustCompile): it
// PANICS on any failure, including invalid input — an apiKey or modelID that is
// not valid UTF-8 or contains a NUL. Use NewOpenAI for anything caller-supplied.
func OpenAI(apiKey, modelID string) *Model {
	return mustNew(NewOpenAI(apiKey, modelID))
}

// OpenAIWithBase creates an OpenAI model with a custom base URL. Must-style: it
// PANICS on any failure, including an apiKey / modelID / baseURL that is not
// valid UTF-8 or contains a NUL. Use NewOpenAIWithBase to get an error instead.
func OpenAIWithBase(apiKey, modelID, baseURL string) *Model {
	return mustNew(NewOpenAIWithBase(apiKey, modelID, baseURL))
}

// InitLogging initializes the global logger (RFC-0014). Idempotent — safe to
// call any number of times; no-op if the host already registered its own
// subscriber. level: "off"|"error"|"warn"|"info"|"debug"|"trace" (empty
// defaults to "warn"). The AIMUX_LOG / AIMUX_LOG_LEVEL env vars take
// precedence. Logs go to stderr.
//
// An unusable level — empty, not valid UTF-8, or containing a NUL — falls back
// to "warn" rather than failing: this has no error channel, and aimux-core
// already treats every unparseable level as "use the default". That matters
// because the obvious call is InitLogging(os.Getenv("AIMUX_LOG_LEVEL")), and
// os.Getenv hands back raw bytes on POSIX.
func InitLogging(level string) {
	if level == "" || checkUTF8("level", level) != nil {
		level = "warn"
	}
	cLevel := C.CString(level)
	defer C.free(unsafe.Pointer(cLevel))
	if err := expectFfiError(C.aimux_init_logging(cLevel)); err != nil {
		// Unreachable: level is now non-NULL, valid UTF-8 and NUL-free, which
		// is every failure aimux_init_logging documents. A non-nil error here
		// is a header/library mismatch, like the enum panics below.
		panic(err)
	}
}

// InitRecording starts RFC-0023 recording: complete Recording JSONL is written
// to {dir}/recordings.jsonl (dir auto-created). Recording is opt-in; calling
// again replaces the recorder. Construction failures are reported as
// *RecordingError (Code RecordingErrorInit when dir cannot be created,
// RecordingErrorOpenFile, RecordingErrorSpawn); on failure the previous
// recorder, if any, stays in place.
func InitRecording(dir string) error {
	if err := checkUTF8("dir", dir); err != nil {
		return err
	}
	cDir := C.CString(dir)
	defer C.free(unsafe.Pointer(cDir))
	return expectRecordingError(C.aimux_init_recording(cDir))
}

// InitRecordingRing starts in-memory bounded recording (RFC-0023 P6): FIFO
// ring with the given capacity.
//
// cap is optional (variadic): omit it to use the library default capacity,
// which calls the FFI aimux_init_recording_ring_default entry point. When
// provided, cap must be > 0 — the C ABI returns `AiMuxError::InvalidArgument`
// for cap == 0. Unlike the previous behavior, this binding no longer
// silently rewrites cap == 0 to 2048; callers must omit cap for the default or
// pass an explicit > 0 capacity. This matches Kotlin/Java (which throw) and
// Swift/Flutter (which surface the C error). Returns an error when cap == 0,
// more than one cap is supplied, or the C call fails.
func InitRecordingRing(cap ...uint64) error {
	if len(cap) == 0 {
		// No cap: library default capacity (FFI default entry point).
		C.aimux_init_recording_ring_default()
		return nil
	}
	if len(cap) > 1 {
		return fmt.Errorf("aimux: InitRecordingRing accepts at most one cap, got %d", len(cap))
	}
	c := cap[0]
	if c == 0 {
		return fmt.Errorf("aimux: InitRecordingRing requires cap > 0 (got 0)")
	}
	return expectAimuxError(C.aimux_init_recording_ring(C.uint64_t(c)))
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

// RecordingTryFlush flushes the global recorder and reports failures as
// *RecordingError (Code RecordingErrorWriterGone, RecordingErrorFlushTimeout
// or RecordingErrorWrite); nil when the JSONL is on disk (or nothing is
// recording). The legacy RecordingFlush stays and never reports.
func RecordingTryFlush() error {
	return expectRecordingError(C.aimux_recording_try_flush())
}

// MockReplay creates a mock replay model from recorded JSONL (RFC-0023 P3):
// it returns recorded responses by input match — no real API is sent. The
// returned model works with GenerateText / StreamText.
func MockReplay(recordingsJsonl string) (*Model, error) {
	// checkJSON per line: UTF-8 / NUL / json.Valid / surrogate pairing, with
	// "" (a blank line) skipped as before.
	for _, line := range strings.Split(recordingsJsonl, "\n") {
		if err := checkJSON("recordings_jsonl", strings.TrimSpace(line)); err != nil {
			return nil, err
		}
	}
	cJsonl := C.CString(recordingsJsonl)
	defer C.free(unsafe.Pointer(cJsonl))
	var h C.uint64_t
	return wrapHandle(&h, C.aimux_mock_replay_new(cJsonl, &h))
}

// RegisterProviders registers external OpenAI-compatible providers from a JSON
// config string (RFC-0020). Entries override same-named built-ins or add new
// ones. configJSON shape: { "providers": [ { "name", "base_url", ... } ] }.
func RegisterProviders(configJSON string) error {
	if err := requireJSON("config_json", configJSON); err != nil {
		return err
	}
	cJSON := C.CString(configJSON)
	defer C.free(unsafe.Pointer(cJSON))
	return expectAimuxError(C.aimux_register_providers(cJSON))
}

// InitProxy sets the global proxy configuration (M6, RFC-0016). Must be called
// before the first GenerateText / StreamText call; a no-op if the shared HTTP
// client is already initialised. configJSON is required (pass "{}" for
// defaults); shape:
// { "http_url": "...", "https_url": "...", "all_url": "...", "no_proxy": "..." }
// (all fields optional).
func InitProxy(configJSON string) error {
	if err := requireJSON("config_json", configJSON); err != nil {
		return err
	}
	cJSON := C.CString(configJSON)
	defer C.free(unsafe.Pointer(cJSON))
	return expectAimuxError(C.aimux_init_proxy(cJSON))
}

// NewRouter builds a RouterModel (RFC-0021) over the given child models. The
// returned model routes each call to one child and falls back across the rest
// on error (per configJSON). models must be non-empty.
//
// configJSON (optional): {"router": "rule"|"weighted", "weights": [...],
// "fallback": "on_error"|"none", "provider_name", "model_id"}.
//
// The same model may appear several times. A nil entry returns an error
// ("aimux: router: models[i] is nil"), never a panic. Concurrent Close is
// resolved by the Rust registry: construction either clones every live child
// it reaches or returns an invalid-handle error.
func NewRouter(models []*Model, configJSON string) (*Model, error) {
	if len(models) == 0 {
		return nil, errors.New("aimux: router needs at least one model")
	}
	if err := checkJSON("config_json", configJSON); err != nil {
		return nil, err
	}
	for i, m := range models {
		if m == nil {
			return nil, fmt.Errorf("aimux: router: models[%d] is nil", i)
		}
	}
	handles, err := modelHandles(models)
	if err != nil {
		return nil, err
	}
	defer keepModelsAlive(models)

	var cJSON *C.char
	if configJSON != "" {
		cJSON = C.CString(configJSON)
		defer C.free(unsafe.Pointer(cJSON))
	}
	var h C.uint64_t
	return wrapHandle(&h, C.aimux_router_new(
		(*C.uint64_t)(unsafe.Pointer(&handles[0])),
		C.size_t(len(handles)),
		cJSON,
		&h,
	))
}

// NewMoa builds a MoaModel (RFC-0022) over reference models + one aggregator.
// References fan out in parallel, then the aggregator synthesizes a final
// answer. references may be empty (runs aggregator only).
//
// configJSON (optional) is a serialized MoaConfig.
//
// The aggregator is allowed to appear in references too, and references may
// repeat. No Go locks are taken; Rust clones every model Arc it resolves. A
// nil aggregator or reference returns an error, never a panic.
func NewMoa(references []*Model, aggregator *Model, configJSON string) (*Model, error) {
	if err := checkJSON("config_json", configJSON); err != nil {
		return nil, err
	}
	if aggregator == nil {
		return nil, errors.New("aimux: moa: aggregator is nil")
	}
	for i, m := range references {
		if m == nil {
			return nil, fmt.Errorf("aimux: moa: references[%d] is nil", i)
		}
	}
	models := append(append(make([]*Model, 0, len(references)+1), references...), aggregator)
	handles, err := modelHandles(models)
	if err != nil {
		return nil, err
	}
	defer keepModelsAlive(models)
	aggHandle := handles[len(references)]

	// No references: NULL pointer + 0 length.
	var refPtr *C.uint64_t
	if len(references) > 0 {
		refPtr = (*C.uint64_t)(unsafe.Pointer(&handles[0]))
	}
	var cJSON *C.char
	if configJSON != "" {
		cJSON = C.CString(configJSON)
		defer C.free(unsafe.Pointer(cJSON))
	}
	var h C.uint64_t
	return wrapHandle(&h, C.aimux_moa_new(
		refPtr,
		C.size_t(len(references)),
		C.uint64_t(aggHandle),
		cJSON,
		&h,
	))
}

// Anthropic creates an Anthropic model instance. Must-style: it PANICS on any
// failure, including an apiKey / modelID that is not valid UTF-8 or contains a
// NUL. Use NewAnthropic to get an error instead.
func Anthropic(apiKey, modelID string) *Model {
	return mustNew(NewAnthropic(apiKey, modelID))
}

// AnthropicWithBase creates an Anthropic model with a custom base URL.
// Must-style: it PANICS on any failure, including an apiKey / modelID /
// baseURL that is not valid UTF-8 or contains a NUL. Use NewAnthropicWithBase
// to get an error instead.
func AnthropicWithBase(apiKey, modelID, baseURL string) *Model {
	return mustNew(NewAnthropicWithBase(apiKey, modelID, baseURL))
}

// mustNew is the Must-style constructor helper shared by OpenAI / OpenAIWithBase /
// Anthropic / AnthropicWithBase / DeepSeek: the NewXxx twin's error becomes a
// panic. Invalid input panics too — that is the documented Go convention
// (regexp.MustCompile), so these five are for compile-time-known arguments.
func mustNew(m *Model, err error) *Model {
	if err != nil {
		panic(err)
	}
	return m
}

// wrapHandle turns an [AiMuxError] constructor's out-handle + error into *Model.
// It takes the out-handle by pointer so it can be written in the same
// expression: wrapHandle(&h, C.aimux_x_new(..., &h)).
func wrapHandle(h *C.uint64_t, e *C.aimux_error_t) (*Model, error) {
	if err := expectAimuxError(e); err != nil {
		return nil, err
	}
	m := &Model{}
	m.id.Store(uint64(*h))
	runtime.SetFinalizer(m, func(m *Model) { m.Close() })
	return m, nil
}

func newModel(apiKey, modelID, baseURL, provider string) (*Model, error) {
	if err := checkUTF8("api_key", apiKey, "model_id", modelID, "base_url", baseURL); err != nil {
		return nil, err
	}
	cKey := C.CString(apiKey)
	cModel := C.CString(modelID)
	defer C.free(unsafe.Pointer(cKey))
	defer C.free(unsafe.Pointer(cModel))

	var h C.uint64_t
	var e *C.aimux_error_t
	if baseURL == "" {
		switch provider {
		case "anthropic":
			e = C.aimux_anthropic_new(cKey, cModel, &h)
		case "cohere":
			e = C.aimux_cohere_new(cKey, cModel, &h)
		case "mistral":
			e = C.aimux_mistral_new(cKey, cModel, &h)
		case "xai":
			e = C.aimux_xai_new(cKey, cModel, &h)
		default:
			e = C.aimux_openai_new(cKey, cModel, &h)
		}
	} else {
		cBase := C.CString(baseURL)
		defer C.free(unsafe.Pointer(cBase))
		switch provider {
		case "anthropic":
			e = C.aimux_anthropic_new_with_base(cKey, cModel, cBase, &h)
		case "cohere":
			e = C.aimux_cohere_new_with_base(cKey, cModel, cBase, &h)
		case "mistral":
			e = C.aimux_mistral_new_with_base(cKey, cModel, cBase, &h)
		case "xai":
			e = C.aimux_xai_new_with_base(cKey, cModel, cBase, &h)
		default:
			e = C.aimux_openai_new_with_base(cKey, cModel, cBase, &h)
		}
	}
	return wrapHandle(&h, e)
}

func newBedrockModel(accessKeyID, secretAccessKey, region, modelID, baseURL string) (*Model, error) {
	if err := checkUTF8("access_key_id", accessKeyID, "secret_access_key", secretAccessKey,
		"region", region, "model_id", modelID, "base_url", baseURL); err != nil {
		return nil, err
	}
	cAccess := C.CString(accessKeyID)
	cSecret := C.CString(secretAccessKey)
	cRegion := C.CString(region)
	cModel := C.CString(modelID)
	defer C.free(unsafe.Pointer(cAccess))
	defer C.free(unsafe.Pointer(cSecret))
	defer C.free(unsafe.Pointer(cRegion))
	defer C.free(unsafe.Pointer(cModel))

	var h C.uint64_t
	if baseURL == "" {
		return wrapHandle(&h, C.aimux_bedrock_new(cAccess, cSecret, cRegion, cModel, &h))
	}
	cBase := C.CString(baseURL)
	defer C.free(unsafe.Pointer(cBase))
	return wrapHandle(&h, C.aimux_bedrock_new_with_base(cAccess, cSecret, cRegion, cModel, cBase, &h))
}

func newVertexModel(accessToken, project, location, modelID, baseURL string) (*Model, error) {
	if err := checkUTF8("access_token", accessToken, "project", project,
		"location", location, "model_id", modelID, "base_url", baseURL); err != nil {
		return nil, err
	}
	cToken := C.CString(accessToken)
	cProject := C.CString(project)
	cLocation := C.CString(location)
	cModel := C.CString(modelID)
	defer C.free(unsafe.Pointer(cToken))
	defer C.free(unsafe.Pointer(cProject))
	defer C.free(unsafe.Pointer(cLocation))
	defer C.free(unsafe.Pointer(cModel))

	var h C.uint64_t
	if baseURL == "" {
		return wrapHandle(&h, C.aimux_vertex_new(cToken, cProject, cLocation, cModel, &h))
	}
	cBase := C.CString(baseURL)
	defer C.free(unsafe.Pointer(cBase))
	return wrapHandle(&h, C.aimux_vertex_new_with_base(cToken, cProject, cLocation, cModel, cBase, &h))
}

func newAnthropicAwsModel(apiKey, region, modelID, baseURL string) (*Model, error) {
	if err := checkUTF8("api_key", apiKey, "region", region,
		"model_id", modelID, "base_url", baseURL); err != nil {
		return nil, err
	}
	cKey := C.CString(apiKey)
	cRegion := C.CString(region)
	cModel := C.CString(modelID)
	defer C.free(unsafe.Pointer(cKey))
	defer C.free(unsafe.Pointer(cRegion))
	defer C.free(unsafe.Pointer(cModel))

	var h C.uint64_t
	if baseURL == "" {
		return wrapHandle(&h, C.aimux_anthropic_aws_new(cKey, cRegion, cModel, &h))
	}
	cBase := C.CString(baseURL)
	defer C.free(unsafe.Pointer(cBase))
	return wrapHandle(&h, C.aimux_anthropic_aws_new_with_base(cKey, cRegion, cModel, cBase, &h))
}

func newAzureModel(apiKey, resourceOrBase, deployment, apiVersion string, useBase bool) (*Model, error) {
	resourceParam := "resource_name"
	if useBase {
		resourceParam = "base_url"
	}
	if err := checkUTF8("api_key", apiKey, resourceParam, resourceOrBase,
		"deployment", deployment, "api_version", apiVersion); err != nil {
		return nil, err
	}
	cKey := C.CString(apiKey)
	cResource := C.CString(resourceOrBase)
	cDeployment := C.CString(deployment)
	cVersion := C.CString(apiVersion)
	defer C.free(unsafe.Pointer(cKey))
	defer C.free(unsafe.Pointer(cResource))
	defer C.free(unsafe.Pointer(cDeployment))
	defer C.free(unsafe.Pointer(cVersion))

	var h C.uint64_t
	if useBase {
		return wrapHandle(&h, C.aimux_azure_new_with_base(cKey, cResource, cDeployment, cVersion, &h))
	}
	return wrapHandle(&h, C.aimux_azure_new(cKey, cResource, cDeployment, cVersion, &h))
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
	if err := checkUTF8("name", name, "api_key", apiKey, "model_id", modelID); err != nil {
		return nil, err
	}
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
		buf, err := marshalJSON("config_json", cfg)
		if err != nil {
			return nil, err
		}
		cConfig = C.CString(buf)
		defer C.free(unsafe.Pointer(cConfig))
	}

	var h C.uint64_t
	return wrapHandle(&h, C.aimux_provider_new(cName, cKey, cModel, cConfig, &h))
}

// ── Provider handles (RFC-0027) ─────────────────────────────────────────────

// ProviderHandle is a provider handle created by CreateProvider. It supports
// ListModels (runtime discovery) and Model (build a model from a discovered id).
// It must not be copied after first use.
type ProviderHandle struct {
	id atomic.Uint64 // 0 means closed
}

// Close releases the native handle. Safe to call multiple times.
func (p *ProviderHandle) Close() error {
	if p == nil {
		return nil
	}
	if id := p.id.Swap(0); id != 0 {
		C.aimux_drop_handle(C.uint64_t(id))
		runtime.SetFinalizer(p, nil)
	}
	return nil
}

func (p *ProviderHandle) handle() (uint64, error) {
	if p != nil {
		if id := p.id.Load(); id != 0 {
			return id, nil
		}
	}
	return 0, fmt.Errorf("%w: provider", ErrClosed)
}

// ListModels lists models available on this provider (runtime discovery via
// the provider's /models endpoint), enriched with community knowledge (anya2a)
// when available. Returns a JSON array of RuntimeModel.
func (p *ProviderHandle) ListModels() (string, error) {
	id, err := p.handle()
	if err != nil {
		return "", err
	}
	defer runtime.KeepAlive(p)
	return ffiString(func(out **C.char) *C.aimux_error_t {
		return C.aimux_provider_list_models(C.uint64_t(id), out)
	})
}

// Model builds a language model from a discovered model id.
func (p *ProviderHandle) Model(modelID string) (*Model, error) {
	if err := checkUTF8("model_id", modelID); err != nil {
		return nil, err
	}
	id, err := p.handle()
	if err != nil {
		return nil, err
	}
	defer runtime.KeepAlive(p)
	cModel := C.CString(modelID)
	defer C.free(unsafe.Pointer(cModel))
	var h C.uint64_t
	return wrapHandle(&h, C.aimux_provider_model(C.uint64_t(id), cModel, &h))
}

// CreateProvider creates a provider handle for a registry-backed provider.
// Unlike Provider (which binds to a single modelID), this returns a handle
// that supports ListModels() and Model().
func CreateProvider(name, apiKey string, cfg *ProviderConfig) (*ProviderHandle, error) {
	if err := checkUTF8("name", name, "api_key", apiKey); err != nil {
		return nil, err
	}
	cName := C.CString(name)
	defer C.free(unsafe.Pointer(cName))

	var cKey *C.char
	if apiKey != "" {
		cKey = C.CString(apiKey)
		defer C.free(unsafe.Pointer(cKey))
	}

	var cConfig *C.char
	if cfg != nil {
		buf, err := marshalJSON("config_json", cfg)
		if err != nil {
			return nil, err
		}
		cConfig = C.CString(buf)
		defer C.free(unsafe.Pointer(cConfig))
	}

	var h C.uint64_t
	if err := expectAimuxError(C.aimux_provider_handle_new(cName, cKey, cConfig, &h)); err != nil {
		return nil, err
	}
	p := &ProviderHandle{}
	p.id.Store(uint64(h))
	runtime.SetFinalizer(p, func(p *ProviderHandle) { p.Close() })
	return p, nil
}

// ── Model specs (RFC-0027) ──────────────────────────────────────────────────

// GetModelSpecs fetches the community model catalogue (anya2a). Returns a JSON
// string representing the Catalogue (provider → model_id → ModelSpec).
// Thin fetch — no caching. sourceURL may be "" for the default endpoint.
func GetModelSpecs(sourceURL string) (string, error) {
	if err := checkUTF8("source_url", sourceURL); err != nil {
		return "", err
	}
	var cURL *C.char
	if sourceURL != "" {
		cURL = C.CString(sourceURL)
		defer C.free(unsafe.Pointer(cURL))
	}
	return ffiString(func(out **C.char) *C.aimux_error_t {
		return C.aimux_get_model_specs(cURL, out)
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
	if err := checkPromptOpts(promptJson, optsJson); err != nil {
		return "", err
	}
	handle, err := m.handle()
	if err != nil {
		return "", err
	}
	defer runtime.KeepAlive(m)

	cPrompt := C.CString(promptJson)
	defer C.free(unsafe.Pointer(cPrompt))

	var cOpts *C.char
	if optsJson != "" {
		cOpts = C.CString(optsJson)
		defer C.free(unsafe.Pointer(cOpts))
	}

	return ffiString(func(out **C.char) *C.aimux_error_t {
		return C.aimux_generate_text(C.uint64_t(handle), cPrompt, cOpts, out)
	})
}

// GenerateObject generates a structured JSON object (M12, RFC-0016).
//
// Same signature as GenerateText; returns the JSON-serialized
// GenerateObjectResult. Pass response_format: { "Json": { ... } } via
// optsJson for schema control; the function applies JSON repair before
// parsing.
func (m *Model) GenerateObject(promptJson, optsJson string) (string, error) {
	if err := checkPromptOpts(promptJson, optsJson); err != nil {
		return "", err
	}
	handle, err := m.handle()
	if err != nil {
		return "", err
	}
	defer runtime.KeepAlive(m)

	cPrompt := C.CString(promptJson)
	defer C.free(unsafe.Pointer(cPrompt))

	var cOpts *C.char
	if optsJson != "" {
		cOpts = C.CString(optsJson)
		defer C.free(unsafe.Pointer(cOpts))
	}

	return ffiString(func(out **C.char) *C.aimux_error_t {
		return C.aimux_generate_object(C.uint64_t(handle), cPrompt, cOpts, out)
	})
}

// ConsumeStreamText consumes a stream to completion and returns the
// aggregated result (M11, RFC-0016). Synchronous (blocks until the stream
// finishes).
//
// Same signature as GenerateText; returns the JSON-serialized
// StreamTextResultAggregated.
func (m *Model) ConsumeStreamText(promptJson, optsJson string) (string, error) {
	if err := checkPromptOpts(promptJson, optsJson); err != nil {
		return "", err
	}
	handle, err := m.handle()
	if err != nil {
		return "", err
	}
	defer runtime.KeepAlive(m)

	cPrompt := C.CString(promptJson)
	defer C.free(unsafe.Pointer(cPrompt))

	var cOpts *C.char
	if optsJson != "" {
		cOpts = C.CString(optsJson)
		defer C.free(unsafe.Pointer(cOpts))
	}

	return ffiString(func(out **C.char) *C.aimux_error_t {
		return C.aimux_consume_stream_text(C.uint64_t(handle), cPrompt, cOpts, out)
	})
}

// GenerateTextAsOpenAI performs non-streaming text generation and returns the
// result as an OpenAI Chat Completion JSON string (RFC-0026).
//
// Same as GenerateText, but the returned JSON is a serialized ChatCompletion
// (OpenAI "chat.completion" object) rather than a GenerateTextResult. Works
// with any provider.
func (m *Model) GenerateTextAsOpenAI(promptJson, optsJson string) (string, error) {
	if err := checkPromptOpts(promptJson, optsJson); err != nil {
		return "", err
	}
	handle, err := m.handle()
	if err != nil {
		return "", err
	}
	defer runtime.KeepAlive(m)

	cPrompt := C.CString(promptJson)
	defer C.free(unsafe.Pointer(cPrompt))

	var cOpts *C.char
	if optsJson != "" {
		cOpts = C.CString(optsJson)
		defer C.free(unsafe.Pointer(cOpts))
	}

	return ffiString(func(out **C.char) *C.aimux_error_t {
		return C.aimux_generate_text_as_openai(C.uint64_t(handle), cPrompt, cOpts, out)
	})
}

// ── Streaming generation ─────────────────────────────────────────────────────

// Stream is a handle to an in-progress or completed stream. It must be fully
// drained or explicitly cancelled. The producer goroutine is the only writer
// of err and the only goroutine that closes parts; observing parts close
// publishes err under the Go memory model.
type Stream struct {
	parts       chan string
	cancelCtx   context.Context
	cancelFn    context.CancelCauseFunc
	err         error
	abortHandle uint64
}

// Parts returns a receive-only channel of StreamPart JSON strings.
// The channel is closed when the stream ends (normally or on error).
func (s *Stream) Parts() <-chan string { return s.parts }

// Err returns any error that occurred during streaming.
// It must be called only after Parts() has closed.
func (s *Stream) Err() error { return s.err }

// Cancel stops this stream. It is safe to call more than once.
func (s *Stream) Cancel() { s.cancel(context.Canceled) }

// cancel uses Context's first-cause-wins, concurrency-safe cancellation. The
// native abort operation is idempotent, so repeated calls need no Go lock.
func (s *Stream) cancel(cause error) {
	if cause == nil {
		cause = context.Canceled
	}
	s.cancelFn(cause)
	if s.abortHandle != 0 {
		C.aimux_abort_signal_abort(C.uint64_t(s.abortHandle))
	}
}

// finish is called exactly once by the producer after the synchronous C
// stream call has returned and all callbacks have left the call stack.
func (s *Stream) finish(err error) {
	if cause := context.Cause(s.cancelCtx); cause != nil {
		err = cause
	}
	s.err = err
	close(s.parts)
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
	return m.startStream(ctx, promptJson, optsJson, false)
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
	return m.startStream(ctx, promptJson, optsJson, true)
}

func (m *Model) startStream(ctx context.Context, promptJson, optsJson string, openAI bool) *Stream {
	if ctx == nil {
		ctx = context.Background()
	}
	abortHandle := uint64(C.aimux_abort_signal_new())
	cancelCtx, cancelFn := context.WithCancelCause(context.Background())
	stream := &Stream{
		parts:       make(chan string, 256),
		cancelCtx:   cancelCtx,
		cancelFn:    cancelFn,
		abortHandle: abortHandle,
	}
	callbackHandle := cgo.NewHandle(stream)

	if cause := context.Cause(ctx); cause != nil {
		stream.cancel(cause)
	}
	stopContext := func() bool { return true }
	if ctx.Done() != nil {
		stopContext = context.AfterFunc(ctx, func() { stream.cancel(context.Cause(ctx)) })
	}

	go func() {
		defer callbackHandle.Delete()
		defer C.aimux_abort_signal_drop(C.uint64_t(abortHandle))
		err := m.runStream(stream, callbackHandle, promptJson, optsJson, openAI)
		// Do not depend on the AfterFunc goroutine winning a scheduling race at
		// the exact moment the native call returns.
		if cause := context.Cause(ctx); cause != nil {
			stream.cancel(cause)
		}
		stopContext()
		stream.finish(err)
	}()

	return stream
}

func (m *Model) runStream(stream *Stream, callbackHandle cgo.Handle, promptJson, optsJson string, openAI bool) error {
	if err := checkPromptOpts(promptJson, optsJson); err != nil {
		return err
	}
	select {
	case <-stream.cancelCtx.Done():
		return context.Cause(stream.cancelCtx)
	default:
	}

	handle, err := m.handle()
	if err != nil {
		return err
	}
	defer runtime.KeepAlive(m)

	cPrompt := C.CString(promptJson)
	defer C.free(unsafe.Pointer(cPrompt))

	var cOpts *C.char
	if optsJson != "" {
		cOpts = C.CString(optsJson)
		defer C.free(unsafe.Pointer(cOpts))
	}

	var ffiErr *C.aimux_error_t
	if openAI {
		ffiErr = C.do_stream_openai(
			C.uint64_t(handle), C.uint64_t(stream.abortHandle),
			cPrompt, cOpts, C.uintptr_t(callbackHandle),
		)
	} else {
		ffiErr = C.do_stream(
			C.uint64_t(handle), C.uint64_t(stream.abortHandle),
			cPrompt, cOpts, C.uintptr_t(callbackHandle),
		)
	}
	return expectAimuxError(ffiErr)
}

// cstr copies and frees an aimux-allocated C string; "" for NULL.
func cstr(p *C.char) string {
	if p == nil {
		return ""
	}
	defer C.aimux_free_string(p)
	return C.GoString(p)
}

// ── Error decoding ───────────────────────────────────────────────────────────
//
// Every fallible C call returns *C.aimux_error_t: nil = success, non-nil
// = failure. The unified code space distinguishes AiMuxError (1..13),
// RecordingError (100..105), and failures detected by the C ABI (200..206).
// The latter collapse to a plain error in Go; no public Go error type is added
// for those implementation failures. Every helper frees the pointer once.

// ffiError reads the owner's message into a plain error; the caller has
// deferred aimux_error_free.
func ffiError(e *C.aimux_error_t) error {
	return fmt.Errorf("aimux: %s", cstr(C.aimux_error_message(e)))
}

// expectFfiError decodes a [C ABI] call: nil is success; anything else is a
// C ABI failure.
func expectFfiError(e *C.aimux_error_t) error {
	if e == nil {
		return nil
	}
	defer C.aimux_error_free(e)
	code := int(C.aimux_error_code(e))
	if !ffiCodeFromC(code) {
		panic(fmt.Sprintf("aimux: expected C ABI failure code, got %d", code))
	}
	return ffiError(e)
}

// expectAimuxError decodes an [AiMuxError] call: nil → nil; 1..13 → *Error;
// 200..206 → plain C ABI error. Any other code is an ABI contract violation.
func expectAimuxError(e *C.aimux_error_t) error {
	if e == nil {
		return nil
	}
	defer C.aimux_error_free(e)
	codeValue := int(C.aimux_error_code(e))
	if ffiCodeFromC(codeValue) {
		return ffiError(e)
	}
	str := cstr
	code, ok := codeFromC(codeValue)
	if !ok {
		panic(fmt.Sprintf("aimux: unknown aimux_error_code_t: %d", codeValue))
	}
	msg := str(C.aimux_error_message(e))
	if msg == "" {
		msg = fmt.Sprintf("aimux: %s", code.String())
	}
	err := &Error{
		Code:      code,
		Message:   msg,
		Status:    -1,
		RetryMs:   -1,
		Retryable: C.aimux_error_retryable(e) != 0,
	}
	switch code {
	case CodeAPICall:
		err.Status = int(C.aimux_error_status(e))
		err.RetryMs = int64(C.aimux_error_retry_ms(e))
		err.ProviderCode = str(C.aimux_error_provider_code(e))
		err.ProviderMessage = str(C.aimux_error_provider_message(e))
		err.RequestID = str(C.aimux_error_request_id(e))
		err.ResponseBody = str(C.aimux_error_response_body(e))
	case CodeNoSuchModel:
		err.ModelID = str(C.aimux_error_model_id(e))
		err.ModelType = str(C.aimux_error_model_type(e))
	case CodeNoSuchProvider:
		err.ProviderID = str(C.aimux_error_provider_id(e))
	}
	// TokenExpired carries a 401 by contract even if C reports -1; every
	// other status is the observed one (ApiCall without a status = no HTTP
	// response was observed). See bindings/node src/error.ts.
	err.Status = defaultStatus(code, err.Status)
	return err
}

// expectRecordingError decodes a [RecordingError] call: nil → nil; 100..105 →
// *RecordingError; 200..206 → plain C ABI error. Any other code is an ABI
// contract violation.
func expectRecordingError(e *C.aimux_error_t) error {
	if e == nil {
		return nil
	}
	defer C.aimux_error_free(e)
	codeValue := int(C.aimux_error_code(e))
	if ffiCodeFromC(codeValue) {
		return ffiError(e)
	}
	code, ok := recordingErrorCodeFromC(codeValue)
	if !ok {
		panic(fmt.Sprintf("aimux: unknown aimux_error_code_t: %d", codeValue))
	}
	return &RecordingError{Code: code, Message: cstr(C.aimux_error_message(e))}
}

// ffiString runs an [AiMuxError] call that writes an aimux-allocated C string to
// its out-parameter, freeing it and mapping the error through expectAimuxError.
func ffiString(call func(out **C.char) *C.aimux_error_t) (string, error) {
	var out *C.char
	if err := expectAimuxError(call(&out)); err != nil {
		return "", err
	}
	return cstr(out), nil
}

// ffiStringWithFfiError is ffiString for a call that only reports C ABI codes.
func ffiStringWithFfiError(call func(out **C.char) *C.aimux_error_t) (string, error) {
	var out *C.char
	if err := expectFfiError(call(&out)); err != nil {
		return "", err
	}
	return cstr(out), nil
}

// ── C→Go callback trampolines (called by trampoline_part/done) ─────────

//export goStreamPart
func goStreamPart(id C.uintptr_t, json *C.char) {
	if json == nil {
		return
	}
	s := cgo.Handle(id).Value().(*Stream)
	select {
	case s.parts <- C.GoString(json):
	case <-s.cancelCtx.Done():
	}
}

// Ensure io.Closer interface is satisfied.
var _ io.Closer = (*Model)(nil)
