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
		return 0, nil, newError(CodeInvalidArgument, "aimux: model already closed")
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
// C function that returns char* and fills AimuxError on failure.
func callFFIString(h *multimodalHandle, fn func(handle C.uint64_t, err *C.AimuxError) *C.char) (string, error) {
	handle, release, err := h.acquire()
	if err != nil {
		return "", err
	}
	defer release()

	return ffiString(func(cerr *C.AimuxError) *C.char {
		return fn(C.uint64_t(handle), cerr)
	})
}

// newMultimodalHandleU64 wraps a constructor handle + AimuxError.
func newMultimodalHandleU64(h C.uint64_t, cerr *C.AimuxError) (*multimodalHandle, error) {
	if h == 0 {
		return nil, errorFromC(cerr)
	}
	return &multimodalHandle{handle: uint64(h)}, nil
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
	plain func(ca, cb *C.char, err *C.AimuxError) C.uint64_t,
	withBase func(ca, cb, cbase *C.char, err *C.AimuxError) C.uint64_t,
) (*multimodalHandle, error) {
	var cerr C.AimuxError
	C.aimux_error_clear(&cerr)
	if baseURL == "" {
		ca, cb, cleanup := cstringPair(apiKey, modelID)
		defer cleanup()
		return newMultimodalHandleU64(plain(ca, cb, &cerr), &cerr)
	}
	ca, cb, cbase, cleanup := cstringTriple(apiKey, modelID, baseURL)
	defer cleanup()
	return newMultimodalHandleU64(withBase(ca, cb, cbase, &cerr), &cerr)
}

// newMultimodalModelFiles is the constructor for the Files manager, which
// takes only an api_key (no model_id) and optionally a base_url.
func newMultimodalModelFiles(
	apiKey, baseURL string,
	plain func(ca *C.char, err *C.AimuxError) C.uint64_t,
	withBase func(ca, cbase *C.char, err *C.AimuxError) C.uint64_t,
) (*multimodalHandle, error) {
	var cerr C.AimuxError
	C.aimux_error_clear(&cerr)
	ca := C.CString(apiKey)
	defer C.free(unsafe.Pointer(ca))
	if baseURL == "" {
		return newMultimodalHandleU64(plain(ca, &cerr), &cerr)
	}
	cbase := C.CString(baseURL)
	defer C.free(unsafe.Pointer(cbase))
	return newMultimodalHandleU64(withBase(ca, cbase, &cerr), &cerr)
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

	return ffiString(func(cerr *C.AimuxError) *C.char {
		return C.aimux_embed(C.uint64_t(handle), cVals, cOpts, cerr)
	})
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
		func(ca, cb *C.char, err *C.AimuxError) C.uint64_t { return C.aimux_openai_embedding_new(ca, cb, err) },
		func(ca, cb, cbase *C.char, err *C.AimuxError) C.uint64_t {
			return C.aimux_openai_embedding_new_with_base(ca, cb, cbase, err)
		},
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
		func(ca, cb *C.char, err *C.AimuxError) C.uint64_t { return C.aimux_cohere_embedding_new(ca, cb, err) },
		func(ca, cb, cbase *C.char, err *C.AimuxError) C.uint64_t {
			return C.aimux_cohere_embedding_new_with_base(ca, cb, cbase, err)
		},
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
		func(ca, cb *C.char, err *C.AimuxError) C.uint64_t { return C.aimux_google_embedding_new(ca, cb, err) },
		func(ca, cb, cbase *C.char, err *C.AimuxError) C.uint64_t {
			return C.aimux_google_embedding_new_with_base(ca, cb, cbase, err)
		},
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
	return callFFIString(m.h, func(handle C.uint64_t, err *C.AimuxError) *C.char {
		cOpts := C.CString(optsJSON)
		defer C.free(unsafe.Pointer(cOpts))
		return C.aimux_speech_generate(handle, cOpts, err)
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
		func(ca, cb *C.char, err *C.AimuxError) C.uint64_t { return C.aimux_openai_speech_new(ca, cb, err) },
		func(ca, cb, cbase *C.char, err *C.AimuxError) C.uint64_t {
			return C.aimux_openai_speech_new_with_base(ca, cb, cbase, err)
		},
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
	return callFFIString(m.h, func(handle C.uint64_t, err *C.AimuxError) *C.char {
		cOpts := C.CString(optsJSON)
		defer C.free(unsafe.Pointer(cOpts))
		return C.aimux_image_generate(handle, cOpts, err)
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
		func(ca, cb *C.char, err *C.AimuxError) C.uint64_t { return C.aimux_openai_image_new(ca, cb, err) },
		func(ca, cb, cbase *C.char, err *C.AimuxError) C.uint64_t {
			return C.aimux_openai_image_new_with_base(ca, cb, cbase, err)
		},
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
		func(ca, cb *C.char, err *C.AimuxError) C.uint64_t { return C.aimux_google_image_new(ca, cb, err) },
		func(ca, cb, cbase *C.char, err *C.AimuxError) C.uint64_t {
			return C.aimux_google_image_new_with_base(ca, cb, cbase, err)
		},
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

	return ffiString(func(cerr *C.AimuxError) *C.char {
		return C.aimux_transcription_generate(C.uint64_t(handle), ca, cb, cOpts, cerr)
	})
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
		func(ca, cb *C.char, err *C.AimuxError) C.uint64_t {
			return C.aimux_openai_transcription_new(ca, cb, err)
		},
		func(ca, cb, cbase *C.char, err *C.AimuxError) C.uint64_t {
			return C.aimux_openai_transcription_new_with_base(ca, cb, cbase, err)
		},
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

	return ffiString(func(cerr *C.AimuxError) *C.char {
		return C.aimux_file_upload(C.uint64_t(handle), ca, cb, cOpts, cerr)
	})
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
		func(ca *C.char, err *C.AimuxError) C.uint64_t { return C.aimux_openai_files_new(ca, err) },
		func(ca, cbase *C.char, err *C.AimuxError) C.uint64_t {
			return C.aimux_openai_files_new_with_base(ca, cbase, err)
		},
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
	return callFFIString(m.h, func(handle C.uint64_t, err *C.AimuxError) *C.char {
		cOpts := C.CString(optsJSON)
		defer C.free(unsafe.Pointer(cOpts))
		return C.aimux_rerank(handle, cOpts, err)
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
		func(ca, cb *C.char, err *C.AimuxError) C.uint64_t { return C.aimux_cohere_reranking_new(ca, cb, err) },
		func(ca, cb, cbase *C.char, err *C.AimuxError) C.uint64_t {
			return C.aimux_cohere_reranking_new_with_base(ca, cb, cbase, err)
		},
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
	return callFFIString(m.h, func(handle C.uint64_t, err *C.AimuxError) *C.char {
		cOpts := C.CString(optsJSON)
		defer C.free(unsafe.Pointer(cOpts))
		return C.aimux_video_generate(handle, cOpts, err)
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
		func(ca, cb *C.char, err *C.AimuxError) C.uint64_t { return C.aimux_google_video_new(ca, cb, err) },
		func(ca, cb, cbase *C.char, err *C.AimuxError) C.uint64_t {
			return C.aimux_google_video_new_with_base(ca, cb, cbase, err)
		},
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
	return callFFIString(m.h, func(handle C.uint64_t, err *C.AimuxError) *C.char {
		cOpts := C.CString(optsJSON)
		defer C.free(unsafe.Pointer(cOpts))
		return C.aimux_search(handle, cOpts, err)
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
	var cerr C.AimuxError
	C.aimux_error_clear(&cerr)
	var h C.uint64_t
	if baseURL == "" {
		h = C.aimux_tavily_search_new(ca, nil, &cerr)
	} else {
		cbase := C.CString(baseURL)
		defer C.free(unsafe.Pointer(cbase))
		h = C.aimux_tavily_search_new_with_base(ca, nil, cbase, &cerr)
	}
	mh, err := newMultimodalHandleU64(h, &cerr)
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

// ── TranscriptionSession (RFC-0028 streaming) ───────────────────────────────

// TranscriptionSession is a live streaming-transcription session (RFC-0028).
// Push audio chunks with PushAudio, mark end-of-audio with InputDone, then
// pull transcription parts (JSON TranscriptionStreamPart) with NextPart.
// Close releases the session (safe and idempotent).
type TranscriptionSession struct {
	mu      sync.Mutex
	session uint64
	closed  bool
}

// InputAudioFormat is the input audio format for streaming transcription
// (RFC-0028): e.g. {"format_type": "audio/pcm", "rate": 24000}.
type InputAudioFormat struct {
	FormatType string  `json:"format_type"`
	Rate       *uint32 `json:"rate,omitempty"`
}

// TranscriptionSessionOpts are the optional session options (RFC-0028):
// input audio format, provider options, headers, include_raw_chunks.
type TranscriptionSessionOpts struct {
	InputAudioFormat *InputAudioFormat          `json:"input_audio_format,omitempty"`
	ProviderOptions  map[string]json.RawMessage `json:"provider_options,omitempty"`
	Headers          map[string]string          `json:"headers,omitempty"`
	IncludeRawChunks *bool                      `json:"include_raw_chunks,omitempty"`
}

// ErrTranscriptionEnded is returned by NextPart when the stream ended
// normally (a Finish part was delivered earlier).
var ErrTranscriptionEnded = errors.New("aimux: transcription stream ended")

// ErrTranscriptionTimeout is returned by NextPart when no part arrived
// within the timeout; the session stays live — call again.
var ErrTranscriptionTimeout = errors.New("aimux: transcription part timeout")

// StartTranscriptionSession starts a streaming transcription session on a
// model that supports do_stream (e.g. OpenAI gpt-realtime-whisper).
func StartTranscriptionSession(model *TranscriptionModel, opts *TranscriptionSessionOpts) (*TranscriptionSession, error) {
	return StartTranscriptionSessionWithAbort(model, opts, 0)
}

// StartTranscriptionSessionWithAbort is StartTranscriptionSession with an
// abort handle (from AbortSignalNew); firing it aborts the session.
func StartTranscriptionSessionWithAbort(model *TranscriptionModel, opts *TranscriptionSessionOpts, abortHandle uint64) (*TranscriptionSession, error) {
	modelHandle, release, err := model.h.acquire()
	if err != nil {
		return nil, err
	}
	defer release()

	optsJSON := cNullOrEmpty(opts)
	ca, cleanup := cstring1(optsJSON)
	defer cleanup()

	var cerr C.AimuxError
	C.aimux_error_clear(&cerr)
	h := C.aimux_transcription_session_new(
		C.uint64_t(modelHandle),
		C.uint64_t(abortHandle),
		ca,
		&cerr,
	)
	if h == 0 {
		return nil, errorFromC(&cerr)
	}
	s := &TranscriptionSession{session: uint64(h)}
	runtime.SetFinalizer(s, func(s *TranscriptionSession) { s.Close() })
	return s, nil
}

// PushAudio pushes one binary audio chunk. Blocks while the internal channel
// is full (backpressure propagation).
func (s *TranscriptionSession) PushAudio(audio []byte) error {
	handle, release, err := s.acquire()
	if err != nil {
		return err
	}
	defer release()

	var ptr *C.uint8_t
	if len(audio) > 0 {
		ptr = (*C.uint8_t)(unsafe.Pointer(&audio[0]))
	}
	var cerr C.AimuxError
	C.aimux_error_clear(&cerr)
	rc := C.aimux_transcription_push_audio(
		C.uint64_t(handle),
		ptr,
		C.size_t(len(audio)),
		&cerr,
	)
	if rc == 0 {
		return errorFromC(&cerr)
	}
	return nil
}

// InputDone signals end-of-audio (idempotent).
func (s *TranscriptionSession) InputDone() error {
	handle, release, err := s.acquire()
	if err != nil {
		return err
	}
	defer release()

	var cerr C.AimuxError
	C.aimux_error_clear(&cerr)
	rc := C.aimux_transcription_input_done(C.uint64_t(handle), &cerr)
	if rc == 0 {
		return errorFromC(&cerr)
	}
	return nil
}

// NextPart pulls the next transcription part (JSON TranscriptionStreamPart).
// Returns ErrTranscriptionEnded when the stream finished normally, and
// ErrTranscriptionTimeout when no part arrived in time (retryable).
// timeoutMs: >0 wait at most; 0 immediate poll; <0 wait indefinitely.
func (s *TranscriptionSession) NextPart(timeoutMs int64) (string, error) {
	handle, release, err := s.acquire()
	if err != nil {
		return "", err
	}
	defer release()

	var cerr C.AimuxError
	C.aimux_error_clear(&cerr)
	ptr := C.aimux_transcription_next_part(
		C.uint64_t(handle),
		C.int64_t(timeoutMs),
		&cerr,
	)
	if ptr == nil {
		if cerr.code == C.AIMUX_OK {
			return "", ErrTranscriptionEnded
		}
		if cerr.code == C.AIMUX_E_TIMEOUT {
			// Free any message then map to the retryable sentinel.
			_ = errorFromC(&cerr)
			return "", ErrTranscriptionTimeout
		}
		return "", errorFromC(&cerr)
	}
	return C.GoString(ptr), nil
}

// Close terminates and releases the session (aborts the driver; idempotent).
func (s *TranscriptionSession) Close() {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.closed {
		return
	}
	s.closed = true
	if s.session != 0 {
		C.aimux_transcription_session_drop(C.uint64_t(s.session))
		s.session = 0
	}
}

func (s *TranscriptionSession) acquire() (uint64, func(), error) {
	s.mu.Lock()
	if s.closed {
		s.mu.Unlock()
		return 0, nil, newError(CodeInvalidArgument, "aimux: transcription session already closed")
	}
	h := s.session
	return h, s.mu.Unlock, nil
}

func cstring1(a string) (*C.char, func()) {
	ca := C.CString(a)
	return ca, func() { C.free(unsafe.Pointer(ca)) }
}

// cNullOrEmpty marshals optional opts to JSON, with "" for nil (defaults).
func cNullOrEmpty(v any) string {
	if v == nil {
		return ""
	}
	b, err := json.Marshal(v)
	if err != nil {
		return ""
	}
	return string(b)
}
