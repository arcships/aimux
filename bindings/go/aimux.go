// Package aimux provides Go bindings for the aimux unified LLM service layer
// (172+ AI providers via a single API).
//
// This is the C ABI path (RFC-0001 §3.2) — same as Swift/Kotlin/Flutter/C.
// Go calls aimux-ffi via cgo, statically linking libaimux_ffi.a. The result is
// a single binary with the Rust core compiled in (no .so/.dll/.dylib to ship).
//
// Design doc: rfc/0011-golang-bindings.md
package aimux

/*
#cgo CFLAGS: -I${SRCDIR}/../../aimux-ffi
#cgo LDFLAGS: -L${SRCDIR}/../../target/release -Wl,-Bstatic -laimux_ffi -Wl,-Bdynamic -lpthread -ldl -lm

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

// do_stream: set thread-local ID, call the blocking stream function, clear ID.
// The callbacks fire synchronously during this call.
static void do_stream(uint64_t handle, const char* prompt, const char* opts, int64_t id) {
    current_stream_id = id;
    aimux_stream_text(handle, prompt, opts,
                      trampoline_part, trampoline_done, trampoline_error);
    current_stream_id = 0;
}

// ── Multimodal constructors and operations (not in aimux-ffi.h yet, but
//    exported as C symbols from libaimux_ffi.a) ────────────────────────────

// Embedding
uint64_t aimux_openai_embedding_new(const char *api_key, const char *model_id);
uint64_t aimux_openai_embedding_new_with_base(const char *api_key, const char *model_id, const char *base_url);
uint64_t aimux_cohere_embedding_new(const char *api_key, const char *model_id);
uint64_t aimux_cohere_embedding_new_with_base(const char *api_key, const char *model_id, const char *base_url);
uint64_t aimux_google_embedding_new(const char *api_key, const char *model_id);
uint64_t aimux_google_embedding_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_embed(uint64_t handle, const char *values_json, const char *opts_json);

// Registry provider (RFC-0017 phase 4): name + optional api_key/env + config JSON
uint64_t aimux_provider_new(const char *name, const char *api_key, const char *model_id, const char *config_json);
uint64_t aimux_provider_from_env(const char *name, const char *model_id);

// Speech (TTS)
uint64_t aimux_openai_speech_new(const char *api_key, const char *model_id);
uint64_t aimux_openai_speech_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_speech_generate(uint64_t handle, const char *opts_json);

// Image
uint64_t aimux_openai_image_new(const char *api_key, const char *model_id);
uint64_t aimux_openai_image_new_with_base(const char *api_key, const char *model_id, const char *base_url);
uint64_t aimux_google_image_new(const char *api_key, const char *model_id);
uint64_t aimux_google_image_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_image_generate(uint64_t handle, const char *opts_json);

// Transcription (STT)
uint64_t aimux_openai_transcription_new(const char *api_key, const char *model_id);
uint64_t aimux_openai_transcription_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_transcription_generate(uint64_t handle, const char *audio_base64, const char *media_type, const char *opts_json);

// Files
uint64_t aimux_openai_files_new(const char *api_key);
uint64_t aimux_openai_files_new_with_base(const char *api_key, const char *base_url);
char *aimux_file_upload(uint64_t handle, const char *data_base64, const char *media_type, const char *opts_json);

// Reranking
uint64_t aimux_cohere_reranking_new(const char *api_key, const char *model_id);
uint64_t aimux_cohere_reranking_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_rerank(uint64_t handle, const char *opts_json);

// Video
uint64_t aimux_google_video_new(const char *api_key, const char *model_id);
uint64_t aimux_google_video_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_video_generate(uint64_t handle, const char *opts_json);

// Search
uint64_t aimux_tavily_search_new(const char *api_key, const char *model_id);
uint64_t aimux_tavily_search_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_search(uint64_t handle, const char *opts_json);
*/
import "C"

import (
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
	return newModel(apiKey, modelID, "", false)
}

// NewOpenAIWithBase creates an OpenAI model with a custom base URL
// (for Ollama, OpenRouter, local proxies, etc.).
func NewOpenAIWithBase(apiKey, modelID, baseURL string) (*Model, error) {
	return newModel(apiKey, modelID, baseURL, false)
}

// NewAnthropic creates an Anthropic model instance.
func NewAnthropic(apiKey, modelID string) (*Model, error) {
	return newModel(apiKey, modelID, "", true)
}

// NewAnthropicWithBase creates an Anthropic model with a custom base URL.
func NewAnthropicWithBase(apiKey, modelID, baseURL string) (*Model, error) {
	return newModel(apiKey, modelID, baseURL, true)
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

func newModel(apiKey, modelID, baseURL string, anthropic bool) (*Model, error) {
	m := &Model{}
	cKey := C.CString(apiKey)
	cModel := C.CString(modelID)
	defer C.free(unsafe.Pointer(cKey))
	defer C.free(unsafe.Pointer(cModel))

	var h C.uint64_t
	if baseURL == "" {
		if anthropic {
			h = C.aimux_anthropic_new(cKey, cModel)
		} else {
			h = C.aimux_openai_new(cKey, cModel)
		}
	} else {
		cBase := C.CString(baseURL)
		defer C.free(unsafe.Pointer(cBase))
		if anthropic {
			h = C.aimux_anthropic_new_with_base(cKey, cModel, cBase)
		} else {
			h = C.aimux_openai_new_with_base(cKey, cModel, cBase)
		}
	}
	if h == 0 {
		return nil, errors.New("aimux: failed to create model (handle=0)")
	}
	m.handle = uint64(h)

	// Set finalizer as a safety net; callers should still use Close().
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
	if baseURL != "" {
		cfg := `{"base_url":"` + baseURL + `"}`
		cConfig = C.CString(cfg)
		defer C.free(unsafe.Pointer(cConfig))
	}

	h := C.aimux_provider_new(cName, cKey, cModel, cConfig)
	if h == 0 {
		return nil, fmt.Errorf("aimux: failed to create provider %q (handle=0)", name)
	}
	m.handle = uint64(h)
	runtime.SetFinalizer(m, func(m *Model) { m.Close() })
	return m, nil
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
	parts chan string
	mu    sync.Mutex
	err   error
	once  sync.Once
}

// streamRegistry maps stream IDs to active stream entries. This avoids passing
// Go pointers into C (cgo pointer rules forbid passing Go memory containing Go
// pointers like channels). The ID is a plain int64_t.
var (
	streamRegMu  sync.Mutex
	streamReg    = make(map[int64]*streamEntry)
	streamNextID int64
)

func registerStream() (*streamEntry, int64) {
	streamRegMu.Lock()
	defer streamRegMu.Unlock()
	streamNextID++
	e := &streamEntry{parts: make(chan string, 256)}
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
	e.once.Do(func() { close(e.parts) })
}

// Stream is a handle to an in-progress or completed stream.
// Consume parts via the Parts() channel; check Err() after the channel closes.
//
// The channel is buffered (256). If the caller stops consuming, the native
// stream will block on the 257th part — always drain the channel or use
// StreamTextContext with a context to cancel.
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
// You MUST drain the Parts() channel (or cancel via StreamTextContext).
// If you stop reading, the native callback blocks once the 256-part buffer
// fills, stalling the stream goroutine and the model.
func (m *Model) StreamText(promptJson, optsJson string) *Stream {
	entry, id := registerStream()

	go func() {
		runtime.LockOSThread()
		defer runtime.UnlockOSThread()
		defer unregisterStream(id)
		// Safety net: ensure the channel is always closed even if the
		// native layer never fires on_done/on_error (defensive against
		// future bugs or panic edges in the FFI layer).
		defer entry.closeParts()

		handle, release, err := m.acquireHandle()
		if err != nil {
			entry.mu.Lock()
			entry.err = err
			entry.mu.Unlock()
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

		C.do_stream(C.uint64_t(handle), cPrompt, cOpts, C.int64_t(id))
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
	e.parts <- C.GoString(json)
}

//export goStreamDone
func goStreamDone(id C.int64_t) {
	e := lookupStream(int64(id))
	if e == nil {
		return
	}
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
	e.mu.Lock()
	e.err = errors.New(msg)
	e.mu.Unlock()
	e.closeParts()
}

// Ensure io.Closer interface is satisfied.
var _ io.Closer = (*Model)(nil)
