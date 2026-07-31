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
*/
import "C"

import (
	"errors"
	"fmt"
	"runtime"
	"sync"
	"sync/atomic"
	"unsafe"
)

// ── Model handle wrapper ───────────────────────────────────────────────────

// Model is a model instance backed by a Rust Arc<dyn LanguageModel>.
//
// It implements io.Closer — you MUST call Close (or use defer) to release the
// native handle and avoid memory leaks.
type Model struct {
	handle uint64
	closed atomic.Bool
}

// Close releases the native handle. Safe to call multiple times.
func (m *Model) Close() error {
	if m.closed.Swap(true) {
		return nil
	}
	if m.handle != 0 {
		C.aimux_drop_handle(C.uint64_t(m.handle))
		m.handle = 0
	}
	return nil
}

// ── Provider constructors ───────────────────────────────────────────────────

// OpenAI creates an OpenAI model instance.
//
//	model := aimux.OpenAI("sk-...", "gpt-4o")
//	defer model.Close()
func OpenAI(apiKey, modelID string) *Model {
	return newModel(apiKey, modelID, "", false)
}

// OpenAIWithBase creates an OpenAI model with a custom base URL
// (for Ollama, OpenRouter, local proxies, etc.).
func OpenAIWithBase(apiKey, modelID, baseURL string) *Model {
	return newModel(apiKey, modelID, baseURL, false)
}

// Anthropic creates an Anthropic model instance.
func Anthropic(apiKey, modelID string) *Model {
	return newModel(apiKey, modelID, "", true)
}

// AnthropicWithBase creates an Anthropic model with a custom base URL.
func AnthropicWithBase(apiKey, modelID, baseURL string) *Model {
	return newModel(apiKey, modelID, baseURL, true)
}

func newModel(apiKey, modelID, baseURL string, anthropic bool) *Model {
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
		// Should not happen with fake keys (provider constructs lazily).
		panic("aimux: failed to create model (handle=0)")
	}
	m.handle = uint64(h)

	// Set finalizer as a safety net; callers should still use Close().
	runtime.SetFinalizer(m, func(m *Model) { m.Close() })
	return m
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
//	result, err := model.GenerateText(`"What is Rust?"`)
func (m *Model) GenerateText(promptJson, optsJson string) (string, error) {
	if m.closed.Load() {
		return "", errors.New("aimux: model already closed")
	}

	cPrompt := C.CString(promptJson)
	defer C.free(unsafe.Pointer(cPrompt))

	var cOpts *C.char
	if optsJson != "" {
		cOpts = C.CString(optsJson)
		defer C.free(unsafe.Pointer(cOpts))
	}

	ptr := C.aimux_generate_text(C.uint64_t(m.handle), cPrompt, cOpts)
	if ptr == nil {
		return "", errors.New("aimux: generate_text returned null")
	}
	defer C.aimux_free_string(ptr)

	result := C.GoString(ptr)
	if len(result) > 10 && result[:9] == `{"error"` {
		// Error response from the engine.
		return "", fmt.Errorf("aimux: %s", result)
	}
	return result, nil
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
//	stream := model.StreamText(`"Write a haiku"`)
//	for part := range stream.Parts() {
//	    fmt.Println(part) // StreamPart JSON
//	}
//	if err := stream.Err(); err != nil {
//	    log.Fatal(err)
//	}
func (m *Model) StreamText(promptJson, optsJson string) *Stream {
	entry, id := registerStream()
	handle := m.handle // snapshot in calling goroutine to avoid data race with Close

	go func() {
		runtime.LockOSThread()
		defer runtime.UnlockOSThread()
		defer unregisterStream(id)

		if m.closed.Load() {
			e := lookupStream(int64(id))
			if e != nil {
				e.mu.Lock()
				e.err = errors.New("aimux: model already closed")
				e.mu.Unlock()
				e.closeParts()
			}
			return
		}

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
	e.mu.Lock()
	e.err = errors.New(C.GoString(err))
	e.mu.Unlock()
	e.closeParts()
}
