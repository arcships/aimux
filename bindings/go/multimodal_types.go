// Typed multimodal data structures mirroring the aimux-core wire format
// (same shapes as the ts-rs generated .ts types in bindings/node/src/types/):
// camelCase field names, unions tagged by "type". These types are
// intentionally lenient on decode (unknown keys ignored, every field optional)
// so future engine additions don't break existing clients.

package aimux

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
)

// Bytes is binary data on the wire: a base64 string or an array of byte
// values. It decodes either form; it encodes as a base64 string.
type Bytes []byte

func (b *Bytes) UnmarshalJSON(data []byte) error {
	if len(data) > 0 && data[0] == '[' {
		var values []uint8
		if err := json.Unmarshal(data, &values); err != nil {
			return err
		}
		*b = values
		return nil
	}
	var text string
	if err := json.Unmarshal(data, &text); err != nil {
		return fmt.Errorf("aimux: bytes must be a base64 string or an array of numbers: %w", err)
	}
	decoded, err := base64.StdEncoding.DecodeString(text)
	if err != nil {
		return fmt.Errorf("aimux: invalid base64: %w", err)
	}
	*b = decoded
	return nil
}

// ── Shared types ────────────────────────────────────────────────────────────

// Warning represents a provider warning.
type Warning = json.RawMessage

// ── Embedding ────────────────────────────────────────────────────────────────

// EmbeddingUsage is token usage for an embedding call (input tokens only).
type EmbeddingUsage struct {
	Tokens uint32 `json:"tokens"`
}

// EmbeddingResponse is provider response metadata for embeddings.
type EmbeddingResponse struct {
	Headers map[string]string `json:"headers,omitempty"`
	Body    any               `json:"body,omitempty"`
}

// EmbeddingResult is the result of an embedding call.
type EmbeddingResult struct {
	Embeddings       [][]float32        `json:"embeddings"`
	Usage            *EmbeddingUsage    `json:"usage,omitempty"`
	ProviderMetadata json.RawMessage    `json:"providerMetadata,omitempty"`
	Response         *EmbeddingResponse `json:"response,omitempty"`
	Warnings         []Warning          `json:"warnings,omitempty"`
}

// EmbeddingCallOptions is the options for an embedding call.
type EmbeddingCallOptions struct {
	Values          []string              `json:"values"`
	MaxRetries      *uint32               `json:"maxRetries,omitempty"`
	Timeout         *TimeoutConfiguration `json:"timeout,omitempty"`
	ProviderOptions jsonObj               `json:"providerOptions"`
	Headers         map[string]string     `json:"headers,omitempty"`
}

// ── Speech (TTS) ──────────────────────────────────────────────────────────────

// AudioData is generated audio: a base64 string or an array of bytes on the
// wire, decoded to bytes.
type AudioData = Bytes

// SpeechRequest is request metadata for speech generation.
type SpeechRequest struct {
	// Core: `body: Option<serde_json::Value>` (speech_model.rs) — the raw
	// request body that was sent, not a prompt string.
	Body json.RawMessage `json:"body,omitempty"`
}

// SpeechResponse is provider response metadata for speech.
type SpeechResponse struct {
	Timestamp *string           `json:"timestamp,omitempty"`
	ModelID   *string           `json:"modelId,omitempty"`
	Headers   map[string]string `json:"headers,omitempty"`
	Body      any               `json:"body,omitempty"`
}

// SpeechResult is the result of a speech generation call.
type SpeechResult struct {
	Audio            AudioData       `json:"audio"`
	Warnings         []Warning       `json:"warnings,omitempty"`
	Request          *SpeechRequest  `json:"request,omitempty"`
	Response         SpeechResponse  `json:"response"`
	ProviderMetadata json.RawMessage `json:"providerMetadata,omitempty"`
}

// SpeechCallOptions is the options for speech generation.
type SpeechCallOptions struct {
	Text            string                `json:"text"`
	Voice           *string               `json:"voice,omitempty"`
	OutputFormat    *string               `json:"outputFormat,omitempty"`
	Instructions    *string               `json:"instructions,omitempty"`
	Speed           *float64              `json:"speed,omitempty"`
	Language        *string               `json:"language,omitempty"`
	MaxRetries      *uint32               `json:"maxRetries,omitempty"`
	Timeout         *TimeoutConfiguration `json:"timeout,omitempty"`
	ProviderOptions jsonObj               `json:"providerOptions"`
	Headers         map[string]string     `json:"headers,omitempty"`
}

// ── Image ─────────────────────────────────────────────────────────────────────

// ImageOutputs is the generated images: base64 strings or byte arrays on the
// wire, decoded to bytes.
type ImageOutputs = []Bytes

// ImageUsage is token usage for image generation (if reported).
type ImageUsage struct {
	InputTokens  *uint32 `json:"inputTokens,omitempty"`
	OutputTokens *uint32 `json:"outputTokens,omitempty"`
	TotalTokens  *uint32 `json:"totalTokens,omitempty"`
}

// ImageResponse is provider response metadata for images.
type ImageResponse struct {
	Timestamp *string           `json:"timestamp,omitempty"`
	ModelID   *string           `json:"modelId,omitempty"`
	Headers   map[string]string `json:"headers,omitempty"`
}

// ImageResult is the result of an image generation call.
type ImageResult struct {
	Images           ImageOutputs    `json:"images"`
	Warnings         []Warning       `json:"warnings,omitempty"`
	ProviderMetadata json.RawMessage `json:"providerMetadata,omitempty"`
	Response         ImageResponse   `json:"response"`
	Usage            *ImageUsage     `json:"usage,omitempty"`
}

// ImageCallOptions is the options for image generation.
type ImageCallOptions struct {
	Prompt          *string               `json:"prompt,omitempty"`
	N               int                   `json:"n"`
	Size            *string               `json:"size,omitempty"`        // "WxH", e.g. "1024x1024"
	AspectRatio     *string               `json:"aspectRatio,omitempty"` // "W:H", e.g. "16:9"
	Seed            *uint64               `json:"seed,omitempty"`
	Files           []json.RawMessage     `json:"files,omitempty"`
	Mask            json.RawMessage       `json:"mask,omitempty"`
	MaxRetries      *uint32               `json:"maxRetries,omitempty"`
	Timeout         *TimeoutConfiguration `json:"timeout,omitempty"`
	ProviderOptions jsonObj               `json:"providerOptions"`
	Headers         map[string]string     `json:"headers,omitempty"`
}

// ── Transcription (STT) ───────────────────────────────────────────────────────

// TranscriptionSegment is a transcript segment with timing.
type TranscriptionSegment struct {
	// StartSecond/EndSecond are required f64 in the core (not Option), so they
	// are values rather than pointers and must always be marshalled.
	Text        string  `json:"text"`
	StartSecond float64 `json:"startSecond"`
	EndSecond   float64 `json:"endSecond"`
}

// TranscriptionRequest is request metadata for transcription.
type TranscriptionRequest struct {
	// Core: `body: Option<String>` (transcription_model.rs) — the raw request
	// HTTP body, JSON stringified.
	Body *string `json:"body,omitempty"`
}

// TranscriptionResponse is provider response metadata for transcription.
type TranscriptionResponse struct {
	Timestamp *string           `json:"timestamp,omitempty"`
	ModelID   *string           `json:"modelId,omitempty"`
	Headers   map[string]string `json:"headers,omitempty"`
	Body      any               `json:"body,omitempty"`
}

// TranscriptionResult is the result of a transcription call.
type TranscriptionResult struct {
	Text              string                 `json:"text"`
	Segments          []TranscriptionSegment `json:"segments,omitempty"`
	Language          *string                `json:"language,omitempty"`
	DurationInSeconds *float64               `json:"durationInSeconds,omitempty"`
	Warnings          []Warning              `json:"warnings,omitempty"`
	Request           *TranscriptionRequest  `json:"request,omitempty"`
	Response          TranscriptionResponse  `json:"response"`
	ProviderMetadata  json.RawMessage        `json:"providerMetadata,omitempty"`
}

// TranscriptionCallOptions is the options for transcription.
type TranscriptionCallOptions struct {
	Audio           Bytes                 `json:"audio"`
	MediaType       string                `json:"mediaType"`
	MaxRetries      *uint32               `json:"maxRetries,omitempty"`
	Timeout         *TimeoutConfiguration `json:"timeout,omitempty"`
	ProviderOptions jsonObj               `json:"providerOptions"`
	Headers         map[string]string     `json:"headers,omitempty"`
}

// ── Reranking ────────────────────────────────────────────────────────────────

// RerankingRank is a single reranked entry.
type RerankingRank struct {
	Index          int     `json:"index"`
	RelevanceScore float64 `json:"relevanceScore"`
}

// RerankingResponse is provider response metadata for reranking.
type RerankingResponse struct {
	ID        *string           `json:"id,omitempty"`
	Timestamp *string           `json:"timestamp,omitempty"`
	ModelID   *string           `json:"modelId,omitempty"`
	Headers   map[string]string `json:"headers,omitempty"`
	Body      any               `json:"body,omitempty"`
}

// RerankingResult is the result of a reranking call.
type RerankingResult struct {
	Ranking          []RerankingRank    `json:"ranking"`
	ProviderMetadata json.RawMessage    `json:"providerMetadata,omitempty"`
	Warnings         []Warning          `json:"warnings,omitempty"`
	Response         *RerankingResponse `json:"response,omitempty"`
}

// RerankingCallOptions is the options for reranking.
type RerankingCallOptions struct {
	Documents       json.RawMessage       `json:"documents"`
	Query           string                `json:"query"`
	TopN            *int                  `json:"topN,omitempty"`
	MaxRetries      *uint32               `json:"maxRetries,omitempty"`
	Timeout         *TimeoutConfiguration `json:"timeout,omitempty"`
	ProviderOptions jsonObj               `json:"providerOptions"`
	Headers         map[string]string     `json:"headers,omitempty"`
}

// ── Video ───────────────────────────────────────────────────────────────────

// VideoData is generated video, tagged by Type: "url" (URL), "base64" or
// "binary" (Data, decoded to bytes).
type VideoData struct {
	Type      string `json:"type"`
	URL       string `json:"url,omitempty"`
	Data      Bytes  `json:"data,omitempty"`
	MediaType string `json:"mediaType"`
}

// VideoResponse is provider response metadata for video.
type VideoResponse struct {
	Timestamp *string           `json:"timestamp,omitempty"`
	ModelID   *string           `json:"modelId,omitempty"`
	Headers   map[string]string `json:"headers,omitempty"`
}

// VideoResult is the result of a video generation call.
type VideoResult struct {
	Videos           []VideoData     `json:"videos"`
	Warnings         []Warning       `json:"warnings,omitempty"`
	ProviderMetadata json.RawMessage `json:"providerMetadata,omitempty"`
	Response         VideoResponse   `json:"response"`
}

// VideoPollOptions overrides Core's pacing for an asynchronous video job.
type VideoPollOptions struct {
	IntervalMS *uint64 `json:"intervalMs,omitempty"`
	TimeoutMS  *uint64 `json:"timeoutMs,omitempty"`
}

// VideoCallOptions is the options for video generation.
type VideoCallOptions struct {
	Prompt          *string               `json:"prompt,omitempty"`
	N               *int                  `json:"n,omitempty"`
	AspectRatio     *string               `json:"aspectRatio,omitempty"` // "W:H", e.g. "16:9"
	Resolution      *string               `json:"resolution,omitempty"`  // "WxH", e.g. "1280x720"
	Duration        *float64              `json:"duration,omitempty"`
	Fps             *float64              `json:"fps,omitempty"`
	Seed            *uint64               `json:"seed,omitempty"`
	Image           json.RawMessage       `json:"image,omitempty"`
	FrameImages     []json.RawMessage     `json:"frameImages,omitempty"`
	InputReferences []json.RawMessage     `json:"inputReferences,omitempty"`
	GenerateAudio   *bool                 `json:"generateAudio,omitempty"`
	MaxRetries      *uint32               `json:"maxRetries,omitempty"`
	Poll            *VideoPollOptions     `json:"poll,omitempty"`
	Timeout         *TimeoutConfiguration `json:"timeout,omitempty"`
	ProviderOptions jsonObj               `json:"providerOptions"`
	Headers         map[string]string     `json:"headers,omitempty"`
}

// ── Search ──────────────────────────────────────────────────────────────────

// SearchResultItem is a single search result.
type SearchResultItem struct {
	Title      *string  `json:"title,omitempty"`
	URL        *string  `json:"url,omitempty"`
	Content    *string  `json:"content,omitempty"`
	RawContent *string  `json:"rawContent,omitempty"`
	Score      *float64 `json:"score,omitempty"`
}

// SearchResponse is provider response metadata for search.
type SearchResponse struct {
	Headers map[string]string `json:"headers,omitempty"`
	Body    any               `json:"body,omitempty"`
}

// SearchResult is the result of a search call.
type SearchResult struct {
	Results          []SearchResultItem `json:"results"`
	Answer           *string            `json:"answer,omitempty"`
	ProviderMetadata json.RawMessage    `json:"providerMetadata,omitempty"`
	Warnings         []Warning          `json:"warnings,omitempty"`
	Response         *SearchResponse    `json:"response,omitempty"`
}

// SearchCallOptions is the options for a search call.
type SearchCallOptions struct {
	Query             string                `json:"query"`
	MaxResults        *int                  `json:"maxResults,omitempty"`
	IncludeRawContent *bool                 `json:"includeRawContent,omitempty"`
	TimeRange         *string               `json:"timeRange,omitempty"`
	IncludeDomains    []string              `json:"includeDomains,omitempty"`
	ExcludeDomains    []string              `json:"excludeDomains,omitempty"`
	MaxRetries        *uint32               `json:"maxRetries,omitempty"`
	Timeout           *TimeoutConfiguration `json:"timeout,omitempty"`
	ProviderOptions   jsonObj               `json:"providerOptions"`
	Headers           map[string]string     `json:"headers,omitempty"`
}

// ── Files ───────────────────────────────────────────────────────────────────

// UploadFileResult is the result of a file upload.
type UploadFileResult struct {
	ProviderReference map[string]string `json:"providerReference"`
	MediaType         *string           `json:"mediaType,omitempty"`
	Filename          *string           `json:"filename,omitempty"`
	ProviderMetadata  json.RawMessage   `json:"providerMetadata,omitempty"`
	Warnings          []Warning         `json:"warnings,omitempty"`
}

// UploadFileCallOptions is the options for a file upload.
type UploadFileCallOptions struct {
	// Data is {"type":"data","data":<base64 or bytes>} or {"type":"text","text":...}.
	Data            json.RawMessage `json:"data"`
	MediaType       string          `json:"mediaType"`
	Filename        *string         `json:"filename,omitempty"`
	ProviderOptions jsonObj         `json:"providerOptions"`
}

// jsonObj is a json.RawMessage that serializes as {} when nil (instead of null).
// This is needed because Rust's SharedProviderOptions (HashMap) requires a map,
// not null, in the JSON wire format.
type jsonObj json.RawMessage

func (o jsonObj) MarshalJSON() ([]byte, error) {
	if o == nil {
		return []byte("{}"), nil
	}
	return []byte(o), nil
}

func (o *jsonObj) UnmarshalJSON(data []byte) error {
	*o = jsonObj(append([]byte(nil), data...))
	return nil
}
