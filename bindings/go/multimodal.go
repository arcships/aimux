// Multimodal API for the Go binding — 8 modality models mirroring Node's
// multimodal.rs and the aimux-ffi C ABI.
//
// Each modality is a Go struct wrapping a native handle (uint64). The handle
// is acquired via a provider-specific constructor (e.g. NewOpenAIEmbedding)
// and released via Close. All C ABI data uses JSON strings (base64
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
	"sync/atomic"
	"unsafe"
)

// ── Shared helpers ───────────────────────────────────────────────────────────

// multimodalHandle is the common state for all multimodal model types. The
// atomic swap makes Close non-blocking: a call that already loaded the handle
// may finish (the Rust registry clones its Arc at entry), while later calls see
// zero and return ErrClosed. No Go lock is held across a blocking C call.
//
// A multimodalHandle must not be copied after first use. Public model wrappers
// embed it so atomic.noCopy also lets go vet flag accidental value copies.
type multimodalHandle struct {
	handle atomic.Uint64
}

func (h *multimodalHandle) load() (uint64, error) {
	handle := h.handle.Load()
	if handle == 0 {
		return 0, fmt.Errorf("%w: model", ErrClosed)
	}
	return handle, nil
}

func (h *multimodalHandle) close() {
	if handle := h.handle.Swap(0); handle != 0 {
		C.aimux_drop_handle(C.uint64_t(handle))
	}
}

func closeMultimodal(owner any, h *multimodalHandle) {
	h.close()
	runtime.SetFinalizer(owner, nil)
	runtime.KeepAlive(owner)
}

// callFFIString is the common pattern for a multimodal call that writes a
// char* out-parameter. owner is kept alive until after C returns so its
// finalizer cannot close the handle while the call is entering the Rust
// registry. Explicit concurrent Close is allowed and may make the call fail;
// it cannot cause use-after-free.
func callFFIString(owner any, h *multimodalHandle, fn func(handle C.uint64_t, out **C.char) *C.aimux_error_t) (string, error) {
	handle, err := h.load()
	if err != nil {
		return "", err
	}

	result, callErr := ffiString(func(out **C.char) *C.aimux_error_t {
		return fn(C.uint64_t(handle), out)
	})
	runtime.KeepAlive(owner)
	return result, callErr
}

// newMultimodalHandle wraps a [C ABI] multimodal constructor's out-handle +
// error (these constructors only store config, so the only failure they can
// carry is a C ABI failure — returned, not panicked). Takes the out-handle by
// pointer so it can be written in the same expression.
func newMultimodalHandle(h *C.uint64_t, e *C.aimux_error_t) (uint64, error) {
	if err := expectFfiError(e); err != nil {
		return 0, err
	}
	return uint64(*h), nil
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
	plain func(ca, cb *C.char, out *C.uint64_t) *C.aimux_error_t,
	withBase func(ca, cb, cbase *C.char, out *C.uint64_t) *C.aimux_error_t,
) (uint64, error) {
	if err := checkUTF8("api_key", apiKey, "model_id", modelID, "base_url", baseURL); err != nil {
		return 0, err
	}
	var h C.uint64_t
	if baseURL == "" {
		ca, cb, cleanup := cstringPair(apiKey, modelID)
		defer cleanup()
		return newMultimodalHandle(&h, plain(ca, cb, &h))
	}
	ca, cb, cbase, cleanup := cstringTriple(apiKey, modelID, baseURL)
	defer cleanup()
	return newMultimodalHandle(&h, withBase(ca, cb, cbase, &h))
}

// newMultimodalModelFiles is the constructor for the Files manager, which
// takes only an api_key (no model_id) and optionally a base_url.
func newMultimodalModelFiles(
	apiKey, baseURL string,
	plain func(ca *C.char, out *C.uint64_t) *C.aimux_error_t,
	withBase func(ca, cbase *C.char, out *C.uint64_t) *C.aimux_error_t,
) (uint64, error) {
	if err := checkUTF8("api_key", apiKey, "base_url", baseURL); err != nil {
		return 0, err
	}
	var h C.uint64_t
	ca := C.CString(apiKey)
	defer C.free(unsafe.Pointer(ca))
	if baseURL == "" {
		return newMultimodalHandle(&h, plain(ca, &h))
	}
	cbase := C.CString(baseURL)
	defer C.free(unsafe.Pointer(cbase))
	return newMultimodalHandle(&h, withBase(ca, cbase, &h))
}

// ── EmbeddingModel ──────────────────────────────────────────────────────────

// EmbeddingModel generates vector embeddings for text.
// An EmbeddingModel must not be copied after first use.
type EmbeddingModel struct {
	h multimodalHandle
}

// Close releases the native handle.
func (m *EmbeddingModel) Close() error {
	closeMultimodal(m, &m.h)
	return nil
}

// Embed generates embeddings for the given text values.
// Returns the JSON-serialized EmbeddingResult.
func (m *EmbeddingModel) Embed(values []string, opts *EmbeddingCallOptions) (string, error) {
	valuesJSON, err := marshalJSON("values", values)
	if err != nil {
		return "", err
	}
	optsJSON := ""
	if opts != nil {
		if optsJSON, err = marshalJSON("opts", opts); err != nil {
			return "", err
		}
	}

	handle, err := m.h.load()
	if err != nil {
		return "", err
	}

	cVals := C.CString(valuesJSON)
	defer C.free(unsafe.Pointer(cVals))
	var cOpts *C.char
	if optsJSON != "" {
		cOpts = C.CString(optsJSON)
		defer C.free(unsafe.Pointer(cOpts))
	}

	result, callErr := ffiString(func(out **C.char) *C.aimux_error_t {
		return C.aimux_embed(C.uint64_t(handle), cVals, cOpts, out)
	})
	runtime.KeepAlive(m)
	return result, callErr
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
		func(ca, cb *C.char, out *C.uint64_t) *C.aimux_error_t {
			return C.aimux_openai_embedding_new(ca, cb, out)
		},
		func(ca, cb, cbase *C.char, out *C.uint64_t) *C.aimux_error_t {
			return C.aimux_openai_embedding_new_with_base(ca, cb, cbase, out)
		},
	)
	if err != nil {
		return nil, err
	}
	m := &EmbeddingModel{}
	m.h.handle.Store(mh)
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
		func(ca, cb *C.char, out *C.uint64_t) *C.aimux_error_t {
			return C.aimux_cohere_embedding_new(ca, cb, out)
		},
		func(ca, cb, cbase *C.char, out *C.uint64_t) *C.aimux_error_t {
			return C.aimux_cohere_embedding_new_with_base(ca, cb, cbase, out)
		},
	)
	if err != nil {
		return nil, err
	}
	m := &EmbeddingModel{}
	m.h.handle.Store(mh)
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
		func(ca, cb *C.char, out *C.uint64_t) *C.aimux_error_t {
			return C.aimux_google_embedding_new(ca, cb, out)
		},
		func(ca, cb, cbase *C.char, out *C.uint64_t) *C.aimux_error_t {
			return C.aimux_google_embedding_new_with_base(ca, cb, cbase, out)
		},
	)
	if err != nil {
		return nil, err
	}
	m := &EmbeddingModel{}
	m.h.handle.Store(mh)
	runtime.SetFinalizer(m, func(m *EmbeddingModel) { m.Close() })
	return m, nil
}

// requiredOptsJSON marshals opts for the FFI entry points whose opts_json is
// REQUIRED (it carries the input): speech / image / rerank / video / search.
// A nil opts is rejected here as a plain error instead of crossing the C
// C ABI as "" (which the header documents as a caller bug → panic).
func requiredOptsJSON[T any](method string, opts *T) (string, error) {
	if opts == nil {
		return "", fmt.Errorf("aimux: %s: opts is required", method)
	}
	return marshalJSON("opts", opts)
}

// ── SpeechModel (TTS) ────────────────────────────────────────────────────────

// SpeechModel converts text to speech audio.
// A SpeechModel must not be copied after first use.
type SpeechModel struct {
	h multimodalHandle
}

func (m *SpeechModel) Close() error { closeMultimodal(m, &m.h); return nil }

// Generate generates speech audio from the given options.
func (m *SpeechModel) Generate(opts *SpeechCallOptions) (string, error) {
	optsJSON, err := requiredOptsJSON("SpeechModel.Generate", opts)
	if err != nil {
		return "", err
	}
	return callFFIString(m, &m.h, func(handle C.uint64_t, out **C.char) *C.aimux_error_t {
		cOpts := C.CString(optsJSON)
		defer C.free(unsafe.Pointer(cOpts))
		return C.aimux_speech_generate(handle, cOpts, out)
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
		func(ca, cb *C.char, out *C.uint64_t) *C.aimux_error_t {
			return C.aimux_openai_speech_new(ca, cb, out)
		},
		func(ca, cb, cbase *C.char, out *C.uint64_t) *C.aimux_error_t {
			return C.aimux_openai_speech_new_with_base(ca, cb, cbase, out)
		},
	)
	if err != nil {
		return nil, err
	}
	m := &SpeechModel{}
	m.h.handle.Store(mh)
	runtime.SetFinalizer(m, func(m *SpeechModel) { m.Close() })
	return m, nil
}

// ── ImageModel ──────────────────────────────────────────────────────────────

// ImageModel generates images from prompts.
// An ImageModel must not be copied after first use.
type ImageModel struct {
	h multimodalHandle
}

func (m *ImageModel) Close() error { closeMultimodal(m, &m.h); return nil }

func (m *ImageModel) Generate(opts *ImageCallOptions) (string, error) {
	optsJSON, err := requiredOptsJSON("ImageModel.Generate", opts)
	if err != nil {
		return "", err
	}
	return callFFIString(m, &m.h, func(handle C.uint64_t, out **C.char) *C.aimux_error_t {
		cOpts := C.CString(optsJSON)
		defer C.free(unsafe.Pointer(cOpts))
		return C.aimux_image_generate(handle, cOpts, out)
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
		func(ca, cb *C.char, out *C.uint64_t) *C.aimux_error_t {
			return C.aimux_openai_image_new(ca, cb, out)
		},
		func(ca, cb, cbase *C.char, out *C.uint64_t) *C.aimux_error_t {
			return C.aimux_openai_image_new_with_base(ca, cb, cbase, out)
		},
	)
	if err != nil {
		return nil, err
	}
	m := &ImageModel{}
	m.h.handle.Store(mh)
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
		func(ca, cb *C.char, out *C.uint64_t) *C.aimux_error_t {
			return C.aimux_google_image_new(ca, cb, out)
		},
		func(ca, cb, cbase *C.char, out *C.uint64_t) *C.aimux_error_t {
			return C.aimux_google_image_new_with_base(ca, cb, cbase, out)
		},
	)
	if err != nil {
		return nil, err
	}
	m := &ImageModel{}
	m.h.handle.Store(mh)
	runtime.SetFinalizer(m, func(m *ImageModel) { m.Close() })
	return m, nil
}

// ── TranscriptionModel (STT) ────────────────────────────────────────────────

// TranscriptionModel converts audio to text.
// A TranscriptionModel must not be copied after first use.
type TranscriptionModel struct {
	h multimodalHandle
}

func (m *TranscriptionModel) Close() error { closeMultimodal(m, &m.h); return nil }

// Generate transcribes audio (base64-encoded) to text.
func (m *TranscriptionModel) Generate(audioBase64, mediaType string, opts *TranscriptionCallOptions) (string, error) {
	if err := checkUTF8("audio_base64", audioBase64, "media_type", mediaType); err != nil {
		return "", err
	}
	optsJSON := ""
	if opts != nil {
		var err error
		if optsJSON, err = marshalJSON("opts", opts); err != nil {
			return "", err
		}
	}

	handle, err := m.h.load()
	if err != nil {
		return "", err
	}

	ca, cb, cc, cleanup := cstringTriple(audioBase64, mediaType, optsJSON)
	defer cleanup()
	var cOpts *C.char
	if optsJSON != "" {
		cOpts = cc
	}

	result, callErr := ffiString(func(out **C.char) *C.aimux_error_t {
		return C.aimux_transcription_generate(C.uint64_t(handle), ca, cb, cOpts, out)
	})
	runtime.KeepAlive(m)
	return result, callErr
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
		func(ca, cb *C.char, out *C.uint64_t) *C.aimux_error_t {
			return C.aimux_openai_transcription_new(ca, cb, out)
		},
		func(ca, cb, cbase *C.char, out *C.uint64_t) *C.aimux_error_t {
			return C.aimux_openai_transcription_new_with_base(ca, cb, cbase, out)
		},
	)
	if err != nil {
		return nil, err
	}
	m := &TranscriptionModel{}
	m.h.handle.Store(mh)
	runtime.SetFinalizer(m, func(m *TranscriptionModel) { m.Close() })
	return m, nil
}

// ── Files ───────────────────────────────────────────────────────────────────

// Files manages file uploads to providers.
// A Files value must not be copied after first use.
type Files struct {
	h multimodalHandle
}

func (f *Files) Close() error { closeMultimodal(f, &f.h); return nil }

// Upload uploads a file (base64-encoded) to the provider.
func (f *Files) Upload(dataBase64, mediaType string, opts *UploadFileCallOptions) (string, error) {
	if err := checkUTF8("data_base64", dataBase64, "media_type", mediaType); err != nil {
		return "", err
	}
	optsJSON := ""
	if opts != nil {
		var err error
		if optsJSON, err = marshalJSON("opts", opts); err != nil {
			return "", err
		}
	}

	handle, err := f.h.load()
	if err != nil {
		return "", err
	}

	ca, cb, cc, cleanup := cstringTriple(dataBase64, mediaType, optsJSON)
	defer cleanup()
	var cOpts *C.char
	if optsJSON != "" {
		cOpts = cc
	}

	result, callErr := ffiString(func(out **C.char) *C.aimux_error_t {
		return C.aimux_file_upload(C.uint64_t(handle), ca, cb, cOpts, out)
	})
	runtime.KeepAlive(f)
	return result, callErr
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
		func(ca *C.char, out *C.uint64_t) *C.aimux_error_t { return C.aimux_openai_files_new(ca, out) },
		func(ca, cbase *C.char, out *C.uint64_t) *C.aimux_error_t {
			return C.aimux_openai_files_new_with_base(ca, cbase, out)
		},
	)
	if err != nil {
		return nil, err
	}
	f := &Files{}
	f.h.handle.Store(mh)
	runtime.SetFinalizer(f, func(f *Files) { f.Close() })
	return f, nil
}

// ── RerankingModel ──────────────────────────────────────────────────────────

// RerankingModel reranks documents by relevance to a query.
// A RerankingModel must not be copied after first use.
type RerankingModel struct {
	h multimodalHandle
}

func (m *RerankingModel) Close() error { closeMultimodal(m, &m.h); return nil }

// Rerank reranks documents against a query.
func (m *RerankingModel) Rerank(opts *RerankingCallOptions) (string, error) {
	optsJSON, err := requiredOptsJSON("RerankingModel.Rerank", opts)
	if err != nil {
		return "", err
	}
	return callFFIString(m, &m.h, func(handle C.uint64_t, out **C.char) *C.aimux_error_t {
		cOpts := C.CString(optsJSON)
		defer C.free(unsafe.Pointer(cOpts))
		return C.aimux_rerank(handle, cOpts, out)
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
		func(ca, cb *C.char, out *C.uint64_t) *C.aimux_error_t {
			return C.aimux_cohere_reranking_new(ca, cb, out)
		},
		func(ca, cb, cbase *C.char, out *C.uint64_t) *C.aimux_error_t {
			return C.aimux_cohere_reranking_new_with_base(ca, cb, cbase, out)
		},
	)
	if err != nil {
		return nil, err
	}
	m := &RerankingModel{}
	m.h.handle.Store(mh)
	runtime.SetFinalizer(m, func(m *RerankingModel) { m.Close() })
	return m, nil
}

// ── VideoModel ──────────────────────────────────────────────────────────────

// VideoModel generates videos from prompts.
// A VideoModel must not be copied after first use.
type VideoModel struct {
	h multimodalHandle
}

func (m *VideoModel) Close() error { closeMultimodal(m, &m.h); return nil }

func (m *VideoModel) Generate(opts *VideoCallOptions) (string, error) {
	optsJSON, err := requiredOptsJSON("VideoModel.Generate", opts)
	if err != nil {
		return "", err
	}
	return callFFIString(m, &m.h, func(handle C.uint64_t, out **C.char) *C.aimux_error_t {
		cOpts := C.CString(optsJSON)
		defer C.free(unsafe.Pointer(cOpts))
		return C.aimux_video_generate(handle, cOpts, out)
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
		func(ca, cb *C.char, out *C.uint64_t) *C.aimux_error_t {
			return C.aimux_google_video_new(ca, cb, out)
		},
		func(ca, cb, cbase *C.char, out *C.uint64_t) *C.aimux_error_t {
			return C.aimux_google_video_new_with_base(ca, cb, cbase, out)
		},
	)
	if err != nil {
		return nil, err
	}
	m := &VideoModel{}
	m.h.handle.Store(mh)
	runtime.SetFinalizer(m, func(m *VideoModel) { m.Close() })
	return m, nil
}

// ── SearchModel ─────────────────────────────────────────────────────────────

// SearchModel performs web search.
// A SearchModel must not be copied after first use.
type SearchModel struct {
	h multimodalHandle
}

func (m *SearchModel) Close() error { closeMultimodal(m, &m.h); return nil }

func (m *SearchModel) Search(opts *SearchCallOptions) (string, error) {
	optsJSON, err := requiredOptsJSON("SearchModel.Search", opts)
	if err != nil {
		return "", err
	}
	return callFFIString(m, &m.h, func(handle C.uint64_t, out **C.char) *C.aimux_error_t {
		cOpts := C.CString(optsJSON)
		defer C.free(unsafe.Pointer(cOpts))
		return C.aimux_search(handle, cOpts, out)
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
	if err := checkUTF8("api_key", apiKey, "base_url", baseURL); err != nil {
		return nil, err
	}
	ca := C.CString(apiKey)
	defer C.free(unsafe.Pointer(ca))
	var h C.uint64_t
	var mh uint64
	var err error
	if baseURL == "" {
		mh, err = newMultimodalHandle(&h, C.aimux_tavily_search_new(ca, nil, &h))
	} else {
		cbase := C.CString(baseURL)
		defer C.free(unsafe.Pointer(cbase))
		mh, err = newMultimodalHandle(&h, C.aimux_tavily_search_new_with_base(ca, nil, cbase, &h))
	}
	if err != nil {
		return nil, err
	}
	m := &SearchModel{}
	m.h.handle.Store(mh)
	runtime.SetFinalizer(m, func(m *SearchModel) { m.Close() })
	return m, nil
}

// ── DeepSeek convenience constructor (registry-backed, RFC-0017 phase 4) ────

// DeepSeek is a convenience constructor for DeepSeek (OpenAI-compatible API).
// Must-style: it PANICS on any failure, including an apiKey / modelID that is
// not valid UTF-8 or contains a NUL. Use NewDeepSeek to get an error instead.
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
//
// A TranscriptionSession must not be copied after first use. Close atomically
// takes its native handle and immediately terminates the native session, so it
// can wake an in-flight NextPart(-1) or a backpressured PushAudio instead of
// waiting for that operation to return first.
type TranscriptionSession struct {
	session atomic.Uint64
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
	if model == nil {
		return nil, errors.New("aimux: transcription model is nil")
	}
	modelHandle, err := model.h.load()
	if err != nil {
		return nil, err
	}

	// nil opts marshals to "null" (the FFI's "use defaults"); a typed nil
	// pointer in an `any` is not == nil, which is why this is not special-cased.
	optsJSON, err := marshalJSON("opts", opts)
	if err != nil {
		return nil, err
	}
	ca, cleanup := cstring1(optsJSON)
	defer cleanup()

	var h C.uint64_t
	callErr := expectAimuxError(C.aimux_transcription_session_new(
		C.uint64_t(modelHandle),
		C.uint64_t(abortHandle),
		ca,
		&h,
	))
	runtime.KeepAlive(model)
	if callErr != nil {
		return nil, callErr
	}
	s := &TranscriptionSession{}
	s.session.Store(uint64(h))
	runtime.SetFinalizer(s, func(s *TranscriptionSession) { s.Close() })
	return s, nil
}

// PushAudio pushes one binary audio chunk. Blocks while the internal channel
// is full (backpressure propagation).
func (s *TranscriptionSession) PushAudio(audio []byte) error {
	handle, err := s.load()
	if err != nil {
		return err
	}
	var ptr *C.uint8_t
	if len(audio) > 0 {
		ptr = (*C.uint8_t)(unsafe.Pointer(&audio[0]))
	}
	callErr := expectAimuxError(C.aimux_transcription_push_audio(
		C.uint64_t(handle),
		ptr,
		C.size_t(len(audio)),
	))
	runtime.KeepAlive(audio)
	runtime.KeepAlive(s)
	return callErr
}

// InputDone signals end-of-audio (idempotent).
func (s *TranscriptionSession) InputDone() error {
	handle, err := s.load()
	if err != nil {
		return err
	}
	callErr := expectFfiError(C.aimux_transcription_input_done(C.uint64_t(handle)))
	runtime.KeepAlive(s)
	return callErr
}

// NextPart pulls the next transcription part (JSON TranscriptionStreamPart).
// Returns ErrTranscriptionEnded when the stream finished normally, and
// ErrTranscriptionTimeout when no part arrived in time (retryable).
// timeoutMs: >0 wait at most; 0 immediate poll; <0 wait indefinitely.
func (s *TranscriptionSession) NextPart(timeoutMs int64) (string, error) {
	handle, err := s.load()
	if err != nil {
		return "", err
	}
	var out *C.char
	var state C.int32_t
	callErr := expectAimuxError(C.aimux_transcription_next_part(
		C.uint64_t(handle),
		C.int64_t(timeoutMs),
		&out,
		&state,
	))
	runtime.KeepAlive(s)
	if callErr != nil {
		return "", callErr
	}
	// Timeout is a poll state, not an error.
	switch state {
	case C.AIMUX_TRANSCRIPTION_NEXT_PART_PART:
		return cstr(out), nil
	case C.AIMUX_TRANSCRIPTION_NEXT_PART_ENDED:
		return "", ErrTranscriptionEnded
	case C.AIMUX_TRANSCRIPTION_NEXT_PART_TIMEOUT:
		return "", ErrTranscriptionTimeout
	default:
		panic(fmt.Sprintf("aimux ffi: unknown aimux_transcription_next_part state: %d", int32(state)))
	}
}

// Close terminates and releases the session (aborts the driver; idempotent).
func (s *TranscriptionSession) Close() {
	if handle := s.session.Swap(0); handle != 0 {
		C.aimux_transcription_session_drop(C.uint64_t(handle))
	}
	runtime.SetFinalizer(s, nil)
	runtime.KeepAlive(s)
}

func (s *TranscriptionSession) load() (uint64, error) {
	handle := s.session.Load()
	if handle == 0 {
		return 0, fmt.Errorf("%w: transcription session", ErrClosed)
	}
	return handle, nil
}

func cstring1(a string) (*C.char, func()) {
	ca := C.CString(a)
	return ca, func() { C.free(unsafe.Pointer(ca)) }
}
