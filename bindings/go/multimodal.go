// Multimodal API for the Go binding — 8 modality models mirroring Node's
// multimodal.rs and the aimux-ffi C ABI.
//
// Each modality is a Go struct wrapping a native handle (uint64). The handle
// is acquired via a provider-specific constructor (e.g. NewOpenAIEmbedding)
// and released via Close. All cross-boundary data uses JSON strings (base64
// for binary), matching the C ABI wire format.

package aimux

/*
#cgo CFLAGS: -I${SRCDIR}/../../aimux-ffi
#cgo linux LDFLAGS: -L${SRCDIR}/../../target/release -Wl,-Bstatic -laimux_ffi -Wl,-Bdynamic -lpthread -ldl -lm
#cgo darwin LDFLAGS: -L${SRCDIR}/../../target/release -laimux_ffi
#cgo windows LDFLAGS: -L${SRCDIR}/../../target/release -Wl,-Bstatic -laimux_ffi -Wl,-Bdynamic -lws2_32 -lbcrypt -lntdll -luserenv

#include <stdint.h>
#include <stdlib.h>
#include "aimux-ffi.h"

// Functions exported by libaimux_ffi.a but not yet declared in aimux-ffi.h.
uint64_t aimux_cohere_reranking_new(const char *api_key, const char *model_id);
uint64_t aimux_cohere_reranking_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_rerank(uint64_t handle, const char *opts_json);
uint64_t aimux_google_video_new(const char *api_key, const char *model_id);
uint64_t aimux_google_video_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_video_generate(uint64_t handle, const char *opts_json);
uint64_t aimux_tavily_search_new(const char *api_key, const char *model_id);
uint64_t aimux_tavily_search_new_with_base(const char *api_key, const char *model_id, const char *base_url);
char *aimux_search(uint64_t handle, const char *opts_json);
*/
import "C"

import (
	"encoding/json"
	"errors"
	"fmt"
	"runtime"
	"sync"
	"unsafe"
)

// ── Shared helpers ───────────────────────────────────────────────────────────

// multimodalHandle is the common structure for all multimodal model types.
// It mirrors Model but is kept separate because the C ABI uses distinct
// handle types (Embedding/Speech/Image/... are not interchangeable).
type multimodalHandle struct {
	mu     sync.RWMutex
	handle uint64
	closed bool
}

func (h *multimodalHandle) acquire() (uint64, func(), error) {
	h.mu.RLock()
	if h.closed {
		h.mu.RUnlock()
		return 0, nil, errors.New("aimux: model already closed")
	}
	return h.handle, h.mu.RUnlock, nil
}

func (h *multimodalHandle) close() {
	h.mu.Lock()
	defer h.mu.Unlock()
	if h.closed {
		return
	}
	h.closed = true
	if h.handle != 0 {
		C.aimux_drop_handle(C.uint64_t(h.handle))
		h.handle = 0
	}
}

// callFFIString is a helper for the common pattern: acquire handle, call a
// C function that returns char*, copy the result, free the C string, check
// for error envelope.
func callFFIString(h *multimodalHandle, fn func(handle C.uint64_t) *C.char) (string, error) {
	handle, release, err := h.acquire()
	if err != nil {
		return "", err
	}
	defer release()

	ptr := fn(C.uint64_t(handle))
	if ptr == nil {
		return "", errors.New("aimux: FFI call returned null")
	}
	defer C.aimux_free_string(ptr)

	result := C.GoString(ptr)
	if msg := extractError(result); msg != "" {
		return "", fmt.Errorf("aimux: %s", msg)
	}
	return result, nil
}

// newMultimodalHandle calls a C constructor and returns a handle wrapper.
func newMultimodalHandle(fn func() C.uint64_t) (*multimodalHandle, error) {
	h := C.uint64_t(fn())
	if h == 0 {
		return nil, errors.New("aimux: failed to create model (handle=0)")
	}
	mh := &multimodalHandle{handle: uint64(h)}
	return mh, nil
}

// cstringPair creates two C strings and returns them with a cleanup func.
func cstringPair(a, b string) (*C.char, *C.char, func()) {
	ca := C.CString(a)
	cb := C.CString(b)
	return ca, cb, func() {
		C.free(unsafe.Pointer(ca))
		C.free(unsafe.Pointer(cb))
	}
}

// cstringTriple creates three C strings and returns them with a cleanup func.
func cstringTriple(a, b, c string) (*C.char, *C.char, *C.char, func()) {
	ca := C.CString(a)
	cb := C.CString(b)
	cc := C.CString(c)
	return ca, cb, cc, func() {
		C.free(unsafe.Pointer(ca))
		C.free(unsafe.Pointer(cb))
		C.free(unsafe.Pointer(cc))
	}
}

// newMultimodalModelWithBase is the common constructor for multimodal models
// that take (api_key, model_id) and optionally a base_url. When baseURL is
// empty, the no-base C constructor is used; otherwise the _with_base variant.
func newMultimodalModelWithBase(
	apiKey, modelID, baseURL string,
	plain func(ca, cb *C.char) C.uint64_t,
	withBase func(ca, cb, cbase *C.char) C.uint64_t,
) (*multimodalHandle, error) {
	if baseURL == "" {
		ca, cb, cleanup := cstringPair(apiKey, modelID)
		defer cleanup()
		return newMultimodalHandle(func() C.uint64_t { return plain(ca, cb) })
	}
	ca, cb, cbase, cleanup := cstringTriple(apiKey, modelID, baseURL)
	defer cleanup()
	return newMultimodalHandle(func() C.uint64_t { return withBase(ca, cb, cbase) })
}

// newMultimodalModelFiles is the constructor for the Files manager, which
// takes only an api_key (no model_id) and optionally a base_url.
func newMultimodalModelFiles(
	apiKey, baseURL string,
	plain func(ca *C.char) C.uint64_t,
	withBase func(ca, cbase *C.char) C.uint64_t,
) (*multimodalHandle, error) {
	ca := C.CString(apiKey)
	defer C.free(unsafe.Pointer(ca))
	if baseURL == "" {
		return newMultimodalHandle(func() C.uint64_t { return plain(ca) })
	}
	cbase := C.CString(baseURL)
	defer C.free(unsafe.Pointer(cbase))
	return newMultimodalHandle(func() C.uint64_t { return withBase(ca, cbase) })
}

// ── EmbeddingModel ──────────────────────────────────────────────────────────

// EmbeddingModel generates vector embeddings for text.
type EmbeddingModel struct {
	h *multimodalHandle
}

// Close releases the native handle.
func (m *EmbeddingModel) Close() error {
	m.h.close()
	return nil
}

// Embed generates embeddings for the given text values.
// Returns the JSON-serialized EmbeddingResult.
func (m *EmbeddingModel) Embed(values []string, opts *EmbeddingCallOptions) (string, error) {
	valuesJSON, err := json.Marshal(values)
	if err != nil {
		return "", fmt.Errorf("aimux: failed to marshal values: %w", err)
	}
	optsJSON := ""
	if opts != nil {
		b, err := json.Marshal(opts)
		if err != nil {
			return "", fmt.Errorf("aimux: failed to marshal opts: %w", err)
		}
		optsJSON = string(b)
	}

	handle, release, err := m.h.acquire()
	if err != nil {
		return "", err
	}
	defer release()

	cVals := C.CString(string(valuesJSON))
	defer C.free(unsafe.Pointer(cVals))
	var cOpts *C.char
	if optsJSON != "" {
		cOpts = C.CString(optsJSON)
		defer C.free(unsafe.Pointer(cOpts))
	}

	ptr := C.aimux_embed(C.uint64_t(handle), cVals, cOpts)
	if ptr == nil {
		return "", errors.New("aimux: embed returned null")
	}
	defer C.aimux_free_string(ptr)

	result := C.GoString(ptr)
	if msg := extractError(result); msg != "" {
		return "", fmt.Errorf("aimux: %s", msg)
	}
	return result, nil
}

// ParseEmbeddingResult parses the JSON string returned by Embed.
func ParseEmbeddingResult(jsonStr string) (*EmbeddingResult, error) {
	var r EmbeddingResult
	if err := json.Unmarshal([]byte(jsonStr), &r); err != nil {
		return nil, fmt.Errorf("aimux: failed to parse EmbeddingResult: %w", err)
	}
	return &r, nil
}

// ── Embedding constructors ───────────────────────────────────────────────────

// NewOpenAIEmbedding creates an OpenAI embedding model (e.g. text-embedding-3-small).
func NewOpenAIEmbedding(apiKey, modelID string) (*EmbeddingModel, error) {
	return NewOpenAIEmbeddingWithBase(apiKey, modelID, "")
}

// NewOpenAIEmbeddingWithBase creates an OpenAI embedding model with a custom base URL.
func NewOpenAIEmbeddingWithBase(apiKey, modelID, baseURL string) (*EmbeddingModel, error) {
	mh, err := newMultimodalModelWithBase(apiKey, modelID, baseURL,
		func(ca, cb *C.char) C.uint64_t { return C.aimux_openai_embedding_new(ca, cb) },
		func(ca, cb, cbase *C.char) C.uint64_t { return C.aimux_openai_embedding_new_with_base(ca, cb, cbase) },
	)
	if err != nil {
		return nil, err
	}
	m := &EmbeddingModel{h: mh}
	runtime.SetFinalizer(m, func(m *EmbeddingModel) { m.Close() })
	return m, nil
}

// NewCohereEmbedding creates a Cohere embedding model (e.g. embed-english-v3.0).
func NewCohereEmbedding(apiKey, modelID string) (*EmbeddingModel, error) {
	return NewCohereEmbeddingWithBase(apiKey, modelID, "")
}

// NewCohereEmbeddingWithBase creates a Cohere embedding model with a custom base URL.
func NewCohereEmbeddingWithBase(apiKey, modelID, baseURL string) (*EmbeddingModel, error) {
	mh, err := newMultimodalModelWithBase(apiKey, modelID, baseURL,
		func(ca, cb *C.char) C.uint64_t { return C.aimux_cohere_embedding_new(ca, cb) },
		func(ca, cb, cbase *C.char) C.uint64_t { return C.aimux_cohere_embedding_new_with_base(ca, cb, cbase) },
	)
	if err != nil {
		return nil, err
	}
	m := &EmbeddingModel{h: mh}
	runtime.SetFinalizer(m, func(m *EmbeddingModel) { m.Close() })
	return m, nil
}

// NewGoogleEmbedding creates a Google embedding model (e.g. gemini-embedding-001).
func NewGoogleEmbedding(apiKey, modelID string) (*EmbeddingModel, error) {
	return NewGoogleEmbeddingWithBase(apiKey, modelID, "")
}

// NewGoogleEmbeddingWithBase creates a Google embedding model with a custom base URL.
func NewGoogleEmbeddingWithBase(apiKey, modelID, baseURL string) (*EmbeddingModel, error) {
	mh, err := newMultimodalModelWithBase(apiKey, modelID, baseURL,
		func(ca, cb *C.char) C.uint64_t { return C.aimux_google_embedding_new(ca, cb) },
		func(ca, cb, cbase *C.char) C.uint64_t { return C.aimux_google_embedding_new_with_base(ca, cb, cbase) },
	)
	if err != nil {
		return nil, err
	}
	m := &EmbeddingModel{h: mh}
	runtime.SetFinalizer(m, func(m *EmbeddingModel) { m.Close() })
	return m, nil
}

// ── SpeechModel (TTS) ────────────────────────────────────────────────────────

// SpeechModel converts text to speech audio.
type SpeechModel struct {
	h *multimodalHandle
}

func (m *SpeechModel) Close() error { m.h.close(); return nil }

// Generate generates speech audio from the given options.
func (m *SpeechModel) Generate(opts *SpeechCallOptions) (string, error) {
	optsJSON := ""
	if opts != nil {
		b, err := json.Marshal(opts)
		if err != nil {
			return "", fmt.Errorf("aimux: failed to marshal opts: %w", err)
		}
		optsJSON = string(b)
	}
	return callFFIString(m.h, func(handle C.uint64_t) *C.char {
		cOpts := C.CString(optsJSON)
		defer C.free(unsafe.Pointer(cOpts))
		return C.aimux_speech_generate(handle, cOpts)
	})
}

func ParseSpeechResult(jsonStr string) (*SpeechResult, error) {
	var r SpeechResult
	if err := json.Unmarshal([]byte(jsonStr), &r); err != nil {
		return nil, fmt.Errorf("aimux: failed to parse SpeechResult: %w", err)
	}
	return &r, nil
}

// NewOpenAISpeech creates an OpenAI speech (TTS) model.
func NewOpenAISpeech(apiKey, modelID string) (*SpeechModel, error) {
	return NewOpenAISpeechWithBase(apiKey, modelID, "")
}

// NewOpenAISpeechWithBase creates an OpenAI speech model with a custom base URL.
func NewOpenAISpeechWithBase(apiKey, modelID, baseURL string) (*SpeechModel, error) {
	mh, err := newMultimodalModelWithBase(apiKey, modelID, baseURL,
		func(ca, cb *C.char) C.uint64_t { return C.aimux_openai_speech_new(ca, cb) },
		func(ca, cb, cbase *C.char) C.uint64_t { return C.aimux_openai_speech_new_with_base(ca, cb, cbase) },
	)
	if err != nil {
		return nil, err
	}
	m := &SpeechModel{h: mh}
	runtime.SetFinalizer(m, func(m *SpeechModel) { m.Close() })
	return m, nil
}

// ── ImageModel ──────────────────────────────────────────────────────────────

// ImageModel generates images from prompts.
type ImageModel struct {
	h *multimodalHandle
}

func (m *ImageModel) Close() error { m.h.close(); return nil }

func (m *ImageModel) Generate(opts *ImageCallOptions) (string, error) {
	optsJSON := ""
	if opts != nil {
		b, err := json.Marshal(opts)
		if err != nil {
			return "", fmt.Errorf("aimux: failed to marshal opts: %w", err)
		}
		optsJSON = string(b)
	}
	return callFFIString(m.h, func(handle C.uint64_t) *C.char {
		cOpts := C.CString(optsJSON)
		defer C.free(unsafe.Pointer(cOpts))
		return C.aimux_image_generate(handle, cOpts)
	})
}

func ParseImageResult(jsonStr string) (*ImageResult, error) {
	var r ImageResult
	if err := json.Unmarshal([]byte(jsonStr), &r); err != nil {
		return nil, fmt.Errorf("aimux: failed to parse ImageResult: %w", err)
	}
	return &r, nil
}

// NewOpenAIImage creates an OpenAI image model (e.g. dall-e-3).
func NewOpenAIImage(apiKey, modelID string) (*ImageModel, error) {
	return NewOpenAIImageWithBase(apiKey, modelID, "")
}

// NewOpenAIImageWithBase creates an OpenAI image model with a custom base URL.
func NewOpenAIImageWithBase(apiKey, modelID, baseURL string) (*ImageModel, error) {
	mh, err := newMultimodalModelWithBase(apiKey, modelID, baseURL,
		func(ca, cb *C.char) C.uint64_t { return C.aimux_openai_image_new(ca, cb) },
		func(ca, cb, cbase *C.char) C.uint64_t { return C.aimux_openai_image_new_with_base(ca, cb, cbase) },
	)
	if err != nil {
		return nil, err
	}
	m := &ImageModel{h: mh}
	runtime.SetFinalizer(m, func(m *ImageModel) { m.Close() })
	return m, nil
}

// NewGoogleImage creates a Google image model (e.g. gemini-2.5-flash-image).
func NewGoogleImage(apiKey, modelID string) (*ImageModel, error) {
	return NewGoogleImageWithBase(apiKey, modelID, "")
}

// NewGoogleImageWithBase creates a Google image model with a custom base URL.
func NewGoogleImageWithBase(apiKey, modelID, baseURL string) (*ImageModel, error) {
	mh, err := newMultimodalModelWithBase(apiKey, modelID, baseURL,
		func(ca, cb *C.char) C.uint64_t { return C.aimux_google_image_new(ca, cb) },
		func(ca, cb, cbase *C.char) C.uint64_t { return C.aimux_google_image_new_with_base(ca, cb, cbase) },
	)
	if err != nil {
		return nil, err
	}
	m := &ImageModel{h: mh}
	runtime.SetFinalizer(m, func(m *ImageModel) { m.Close() })
	return m, nil
}

// ── TranscriptionModel (STT) ────────────────────────────────────────────────

// TranscriptionModel converts audio to text.
type TranscriptionModel struct {
	h *multimodalHandle
}

func (m *TranscriptionModel) Close() error { m.h.close(); return nil }

// Generate transcribes audio (base64-encoded) to text.
func (m *TranscriptionModel) Generate(audioBase64, mediaType string, opts *TranscriptionCallOptions) (string, error) {
	optsJSON := ""
	if opts != nil {
		b, err := json.Marshal(opts)
		if err != nil {
			return "", fmt.Errorf("aimux: failed to marshal opts: %w", err)
		}
		optsJSON = string(b)
	}

	handle, release, err := m.h.acquire()
	if err != nil {
		return "", err
	}
	defer release()

	ca, cb, cc, cleanup := cstringTriple(audioBase64, mediaType, optsJSON)
	defer cleanup()
	var cOpts *C.char
	if optsJSON != "" {
		cOpts = cc
	}

	ptr := C.aimux_transcription_generate(C.uint64_t(handle), ca, cb, cOpts)
	if ptr == nil {
		return "", errors.New("aimux: transcription_generate returned null")
	}
	defer C.aimux_free_string(ptr)

	result := C.GoString(ptr)
	if msg := extractError(result); msg != "" {
		return "", fmt.Errorf("aimux: %s", msg)
	}
	return result, nil
}

func ParseTranscriptionResult(jsonStr string) (*TranscriptionResult, error) {
	var r TranscriptionResult
	if err := json.Unmarshal([]byte(jsonStr), &r); err != nil {
		return nil, fmt.Errorf("aimux: failed to parse TranscriptionResult: %w", err)
	}
	return &r, nil
}

// NewOpenAITranscription creates an OpenAI transcription (STT) model.
func NewOpenAITranscription(apiKey, modelID string) (*TranscriptionModel, error) {
	return NewOpenAITranscriptionWithBase(apiKey, modelID, "")
}

// NewOpenAITranscriptionWithBase creates an OpenAI transcription model with a custom base URL.
func NewOpenAITranscriptionWithBase(apiKey, modelID, baseURL string) (*TranscriptionModel, error) {
	mh, err := newMultimodalModelWithBase(apiKey, modelID, baseURL,
		func(ca, cb *C.char) C.uint64_t { return C.aimux_openai_transcription_new(ca, cb) },
		func(ca, cb, cbase *C.char) C.uint64_t { return C.aimux_openai_transcription_new_with_base(ca, cb, cbase) },
	)
	if err != nil {
		return nil, err
	}
	m := &TranscriptionModel{h: mh}
	runtime.SetFinalizer(m, func(m *TranscriptionModel) { m.Close() })
	return m, nil
}

// ── Files ───────────────────────────────────────────────────────────────────

// Files manages file uploads to providers.
type Files struct {
	h *multimodalHandle
}

func (f *Files) Close() error { f.h.close(); return nil }

// Upload uploads a file (base64-encoded) to the provider.
func (f *Files) Upload(dataBase64, mediaType string, opts *UploadFileCallOptions) (string, error) {
	optsJSON := ""
	if opts != nil {
		b, err := json.Marshal(opts)
		if err != nil {
			return "", fmt.Errorf("aimux: failed to marshal opts: %w", err)
		}
		optsJSON = string(b)
	}

	handle, release, err := f.h.acquire()
	if err != nil {
		return "", err
	}
	defer release()

	ca, cb, cc, cleanup := cstringTriple(dataBase64, mediaType, optsJSON)
	defer cleanup()
	var cOpts *C.char
	if optsJSON != "" {
		cOpts = cc
	}

	ptr := C.aimux_file_upload(C.uint64_t(handle), ca, cb, cOpts)
	if ptr == nil {
		return "", errors.New("aimux: file_upload returned null")
	}
	defer C.aimux_free_string(ptr)

	result := C.GoString(ptr)
	if msg := extractError(result); msg != "" {
		return "", fmt.Errorf("aimux: %s", msg)
	}
	return result, nil
}

func ParseUploadFileResult(jsonStr string) (*UploadFileResult, error) {
	var r UploadFileResult
	if err := json.Unmarshal([]byte(jsonStr), &r); err != nil {
		return nil, fmt.Errorf("aimux: failed to parse UploadFileResult: %w", err)
	}
	return &r, nil
}

// NewOpenAIFiles creates an OpenAI files manager.
func NewOpenAIFiles(apiKey string) (*Files, error) {
	return NewOpenAIFilesWithBase(apiKey, "")
}

// NewOpenAIFilesWithBase creates an OpenAI files manager with a custom base URL.
func NewOpenAIFilesWithBase(apiKey, baseURL string) (*Files, error) {
	mh, err := newMultimodalModelFiles(apiKey, baseURL,
		func(ca *C.char) C.uint64_t { return C.aimux_openai_files_new(ca) },
		func(ca, cbase *C.char) C.uint64_t { return C.aimux_openai_files_new_with_base(ca, cbase) },
	)
	if err != nil {
		return nil, err
	}
	f := &Files{h: mh}
	runtime.SetFinalizer(f, func(f *Files) { f.Close() })
	return f, nil
}

// ── RerankingModel ──────────────────────────────────────────────────────────

// RerankingModel reranks documents by relevance to a query.
type RerankingModel struct {
	h *multimodalHandle
}

func (m *RerankingModel) Close() error { m.h.close(); return nil }

// Rerank reranks documents against a query.
func (m *RerankingModel) Rerank(opts *RerankingCallOptions) (string, error) {
	optsJSON := ""
	if opts != nil {
		b, err := json.Marshal(opts)
		if err != nil {
			return "", fmt.Errorf("aimux: failed to marshal opts: %w", err)
		}
		optsJSON = string(b)
	}
	return callFFIString(m.h, func(handle C.uint64_t) *C.char {
		cOpts := C.CString(optsJSON)
		defer C.free(unsafe.Pointer(cOpts))
		return C.aimux_rerank(handle, cOpts)
	})
}

func ParseRerankingResult(jsonStr string) (*RerankingResult, error) {
	var r RerankingResult
	if err := json.Unmarshal([]byte(jsonStr), &r); err != nil {
		return nil, fmt.Errorf("aimux: failed to parse RerankingResult: %w", err)
	}
	return &r, nil
}

// NewCohereReranking creates a Cohere reranking model (e.g. rerank-v3.0).
func NewCohereReranking(apiKey, modelID string) (*RerankingModel, error) {
	return NewCohereRerankingWithBase(apiKey, modelID, "")
}

// NewCohereRerankingWithBase creates a Cohere reranking model with a custom base URL.
func NewCohereRerankingWithBase(apiKey, modelID, baseURL string) (*RerankingModel, error) {
	mh, err := newMultimodalModelWithBase(apiKey, modelID, baseURL,
		func(ca, cb *C.char) C.uint64_t { return C.aimux_cohere_reranking_new(ca, cb) },
		func(ca, cb, cbase *C.char) C.uint64_t { return C.aimux_cohere_reranking_new_with_base(ca, cb, cbase) },
	)
	if err != nil {
		return nil, err
	}
	m := &RerankingModel{h: mh}
	runtime.SetFinalizer(m, func(m *RerankingModel) { m.Close() })
	return m, nil
}

// ── VideoModel ──────────────────────────────────────────────────────────────

// VideoModel generates videos from prompts.
type VideoModel struct {
	h *multimodalHandle
}

func (m *VideoModel) Close() error { m.h.close(); return nil }

func (m *VideoModel) Generate(opts *VideoCallOptions) (string, error) {
	optsJSON := ""
	if opts != nil {
		b, err := json.Marshal(opts)
		if err != nil {
			return "", fmt.Errorf("aimux: failed to marshal opts: %w", err)
		}
		optsJSON = string(b)
	}
	return callFFIString(m.h, func(handle C.uint64_t) *C.char {
		cOpts := C.CString(optsJSON)
		defer C.free(unsafe.Pointer(cOpts))
		return C.aimux_video_generate(handle, cOpts)
	})
}

func ParseVideoResult(jsonStr string) (*VideoResult, error) {
	var r VideoResult
	if err := json.Unmarshal([]byte(jsonStr), &r); err != nil {
		return nil, fmt.Errorf("aimux: failed to parse VideoResult: %w", err)
	}
	return &r, nil
}

// NewGoogleVideo creates a Google video model (e.g. veo-3.0).
func NewGoogleVideo(apiKey, modelID string) (*VideoModel, error) {
	return NewGoogleVideoWithBase(apiKey, modelID, "")
}

// NewGoogleVideoWithBase creates a Google video model with a custom base URL.
func NewGoogleVideoWithBase(apiKey, modelID, baseURL string) (*VideoModel, error) {
	mh, err := newMultimodalModelWithBase(apiKey, modelID, baseURL,
		func(ca, cb *C.char) C.uint64_t { return C.aimux_google_video_new(ca, cb) },
		func(ca, cb, cbase *C.char) C.uint64_t { return C.aimux_google_video_new_with_base(ca, cb, cbase) },
	)
	if err != nil {
		return nil, err
	}
	m := &VideoModel{h: mh}
	runtime.SetFinalizer(m, func(m *VideoModel) { m.Close() })
	return m, nil
}

// ── SearchModel ─────────────────────────────────────────────────────────────

// SearchModel performs web search.
type SearchModel struct {
	h *multimodalHandle
}

func (m *SearchModel) Close() error { m.h.close(); return nil }

func (m *SearchModel) Search(opts *SearchCallOptions) (string, error) {
	optsJSON := ""
	if opts != nil {
		b, err := json.Marshal(opts)
		if err != nil {
			return "", fmt.Errorf("aimux: failed to marshal opts: %w", err)
		}
		optsJSON = string(b)
	}
	return callFFIString(m.h, func(handle C.uint64_t) *C.char {
		cOpts := C.CString(optsJSON)
		defer C.free(unsafe.Pointer(cOpts))
		return C.aimux_search(handle, cOpts)
	})
}

func ParseSearchResult(jsonStr string) (*SearchResult, error) {
	var r SearchResult
	if err := json.Unmarshal([]byte(jsonStr), &r); err != nil {
		return nil, fmt.Errorf("aimux: failed to parse SearchResult: %w", err)
	}
	return &r, nil
}

// NewTavilySearch creates a Tavily search model. Tavily uses a fixed endpoint,
// so no model ID is needed.
func NewTavilySearch(apiKey string) (*SearchModel, error) {
	return NewTavilySearchWithBase(apiKey, "")
}

// NewTavilySearchWithBase creates a Tavily search model with a custom base URL
// (for testing against a mock server).
func NewTavilySearchWithBase(apiKey, baseURL string) (*SearchModel, error) {
	ca := C.CString(apiKey)
	defer C.free(unsafe.Pointer(ca))
	if baseURL == "" {
		mh, err := newMultimodalHandle(func() C.uint64_t {
			return C.aimux_tavily_search_new(ca, nil)
		})
		if err != nil {
			return nil, err
		}
		m := &SearchModel{h: mh}
		runtime.SetFinalizer(m, func(m *SearchModel) { m.Close() })
		return m, nil
	}
	cbase := C.CString(baseURL)
	defer C.free(unsafe.Pointer(cbase))
	mh, err := newMultimodalHandle(func() C.uint64_t {
		return C.aimux_tavily_search_new_with_base(ca, nil, cbase)
	})
	if err != nil {
		return nil, err
	}
	m := &SearchModel{h: mh}
	runtime.SetFinalizer(m, func(m *SearchModel) { m.Close() })
	return m, nil
}

// ── DeepSeek convenience constructor (registry-backed, RFC-0017 phase 4) ────

// DeepSeek is a convenience constructor for DeepSeek (OpenAI-compatible API).
func DeepSeek(apiKey, modelID string) *Model {
	return mustNew(NewDeepSeek(apiKey, modelID))
}

// NewDeepSeek is the error-returning variant of DeepSeek.
func NewDeepSeek(apiKey, modelID string) (*Model, error) {
	return Provider("deepseek", apiKey, modelID)
}
