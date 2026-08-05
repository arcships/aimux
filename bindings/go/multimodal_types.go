// Typed multimodal data structures mirroring the aimux-core wire format
// (same shapes as the ts-rs generated .ts types in aimux-core/bindings/).
//
// Field names use JSON tags matching the wire format's snake_case. These types
// are intentionally lenient on decode (unknown keys ignored, every field
// optional) so future engine additions don't break existing clients.

package aimux

import "encoding/json"

// ── Shared types ────────────────────────────────────────────────────────────

// Warning represents a provider warning.
type Warning = json.RawMessage

// ── Embedding ────────────────────────────────────────────────────────────────

// EmbeddingUsage is token usage for an embedding call (input tokens only).
type EmbeddingUsage struct {
	Tokens *uint32 `json:"tokens,omitempty"`
}

// EmbeddingResponse is provider response metadata for embeddings.
type EmbeddingResponse struct {
	ID        *string `json:"id,omitempty"`
	ModelID   *string `json:"model_id,omitempty"`
	Timestamp *string `json:"timestamp,omitempty"`
}

// EmbeddingResult is the result of an embedding call.
type EmbeddingResult struct {
	Embeddings       [][]float32        `json:"embeddings"`
	Usage            *EmbeddingUsage    `json:"usage,omitempty"`
	ProviderMetadata json.RawMessage    `json:"provider_metadata,omitempty"`
	Response         *EmbeddingResponse `json:"response,omitempty"`
	Warnings         []Warning          `json:"warnings,omitempty"`
}

// EmbeddingCallOptions is the options for an embedding call.
type EmbeddingCallOptions struct {
	Values          []string          `json:"values,omitempty"`
	ProviderOptions jsonObj           `json:"provider_options"`
	Headers         map[string]string `json:"headers,omitempty"`
}

// ── Speech (TTS) ──────────────────────────────────────────────────────────────

// AudioData is generated audio: base64 string or raw binary bytes.
// Wire format: {"Base64": "string"} | {"Binary": [numbers]}
type AudioData struct {
	Base64 *string `json:"Base64,omitempty"`
	Binary []uint8 `json:"Binary,omitempty"`
}

// SpeechRequest is request metadata for speech generation.
type SpeechRequest struct {
	Prompt *string `json:"prompt,omitempty"`
}

// SpeechResponse is provider response metadata for speech.
type SpeechResponse struct {
	ID        *string `json:"id,omitempty"`
	ModelID   *string `json:"model_id,omitempty"`
	Timestamp *string `json:"timestamp,omitempty"`
}

// SpeechResult is the result of a speech generation call.
type SpeechResult struct {
	Audio            AudioData       `json:"audio"`
	Warnings         []Warning       `json:"warnings,omitempty"`
	Request          *SpeechRequest  `json:"request,omitempty"`
	Response         SpeechResponse  `json:"response"`
	ProviderMetadata json.RawMessage `json:"provider_metadata,omitempty"`
}

// SpeechCallOptions is the options for speech generation.
type SpeechCallOptions struct {
	Text            string            `json:"text"`
	Voice           *string           `json:"voice,omitempty"`
	OutputFormat    *string           `json:"output_format,omitempty"`
	Instructions    *string           `json:"instructions,omitempty"`
	Speed           *float64          `json:"speed,omitempty"`
	Language        *string           `json:"language,omitempty"`
	ProviderOptions jsonObj           `json:"provider_options"`
	Headers         map[string]string `json:"headers,omitempty"`
}

// ── Image ─────────────────────────────────────────────────────────────────────

// ImageOutputs is the generated images: all base64 or all binary.
type ImageOutputs struct {
	Base64 []string  `json:"Base64,omitempty"`
	Binary [][]uint8 `json:"Binary,omitempty"`
}

// ImageUsage is token usage for image generation (if reported).
type ImageUsage struct {
	Steps *uint32 `json:"steps,omitempty"`
}

// ImageResponse is provider response metadata for images.
type ImageResponse struct {
	ID        *string `json:"id,omitempty"`
	ModelID   *string `json:"model_id,omitempty"`
	Timestamp *string `json:"timestamp,omitempty"`
}

// ImageResult is the result of an image generation call.
type ImageResult struct {
	Images           ImageOutputs    `json:"images"`
	Warnings         []Warning       `json:"warnings,omitempty"`
	ProviderMetadata json.RawMessage `json:"provider_metadata,omitempty"`
	Response         ImageResponse   `json:"response"`
	Usage            *ImageUsage     `json:"usage,omitempty"`
}

// ImageCallOptions is the options for image generation.
type ImageCallOptions struct {
	Prompt          *string           `json:"prompt,omitempty"`
	N               *int              `json:"n,omitempty"`
	Size            *string           `json:"size,omitempty"`
	AspectRatio     *string           `json:"aspect_ratio,omitempty"`
	Seed            *uint64           `json:"seed,omitempty"`
	Files           []json.RawMessage `json:"files,omitempty"`
	Mask            json.RawMessage   `json:"mask,omitempty"`
	ProviderOptions jsonObj           `json:"provider_options"`
	Headers         map[string]string `json:"headers,omitempty"`
}

// ── Transcription (STT) ───────────────────────────────────────────────────────

// TranscriptionSegment is a transcript segment with timing.
type TranscriptionSegment struct {
	Text  string   `json:"text"`
	Start *float64 `json:"start,omitempty"`
	End   *float64 `json:"end,omitempty"`
}

// TranscriptionRequest is request metadata for transcription.
type TranscriptionRequest struct {
	Audio     json.RawMessage `json:"audio,omitempty"`
	MediaType *string         `json:"media_type,omitempty"`
}

// TranscriptionResponse is provider response metadata for transcription.
type TranscriptionResponse struct {
	ID        *string `json:"id,omitempty"`
	ModelID   *string `json:"model_id,omitempty"`
	Timestamp *string `json:"timestamp,omitempty"`
}

// TranscriptionResult is the result of a transcription call.
type TranscriptionResult struct {
	Text              string                 `json:"text"`
	Segments          []TranscriptionSegment `json:"segments,omitempty"`
	Language          *string                `json:"language,omitempty"`
	DurationInSeconds *float64               `json:"duration_in_seconds,omitempty"`
	Warnings          []Warning              `json:"warnings,omitempty"`
	Request           *TranscriptionRequest  `json:"request,omitempty"`
	Response          TranscriptionResponse  `json:"response"`
	ProviderMetadata  json.RawMessage        `json:"provider_metadata,omitempty"`
}

// TranscriptionCallOptions is the options for transcription.
type TranscriptionCallOptions struct {
	Audio           json.RawMessage   `json:"audio"`
	MediaType       string            `json:"media_type"`
	ProviderOptions jsonObj           `json:"provider_options"`
	Headers         map[string]string `json:"headers,omitempty"`
}

// ── Reranking ────────────────────────────────────────────────────────────────

// RerankingRank is a single reranked entry.
type RerankingRank struct {
	Index          int     `json:"index"`
	RelevanceScore float64 `json:"relevance_score"`
}

// RerankingResponse is provider response metadata for reranking.
type RerankingResponse struct {
	ID        *string `json:"id,omitempty"`
	ModelID   *string `json:"model_id,omitempty"`
	Timestamp *string `json:"timestamp,omitempty"`
}

// RerankingResult is the result of a reranking call.
type RerankingResult struct {
	Ranking          []RerankingRank    `json:"ranking"`
	ProviderMetadata json.RawMessage    `json:"provider_metadata,omitempty"`
	Warnings         []Warning          `json:"warnings,omitempty"`
	Response         *RerankingResponse `json:"response,omitempty"`
}

// RerankingCallOptions is the options for reranking.
type RerankingCallOptions struct {
	Documents       json.RawMessage   `json:"documents"`
	Query           string            `json:"query"`
	TopN            *int              `json:"top_n,omitempty"`
	ProviderOptions jsonObj           `json:"provider_options"`
	Headers         map[string]string `json:"headers,omitempty"`
}

// ── Video ───────────────────────────────────────────────────────────────────

// VideoData is generated video: URL, base64, or binary.
// Wire format: {"Url": {"url":"...", "media_type":"..."}} | {"Base64": {...}} | {"Binary": {...}}
type VideoData struct {
	Url    *VideoUrlData    `json:"Url,omitempty"`
	Base64 *VideoBase64Data `json:"Base64,omitempty"`
	Binary *VideoBinaryData `json:"Binary,omitempty"`
}

// VideoUrlData is the URL variant of VideoData.
type VideoUrlData struct {
	URL       string `json:"url"`
	MediaType string `json:"media_type,omitempty"`
}

// VideoBase64Data is the base64 variant of VideoData.
type VideoBase64Data struct {
	Data      string `json:"data"`
	MediaType string `json:"media_type,omitempty"`
}

// VideoBinaryData is the binary variant of VideoData.
type VideoBinaryData struct {
	Data      []uint8 `json:"data"`
	MediaType string  `json:"media_type,omitempty"`
}

// VideoResponse is provider response metadata for video.
type VideoResponse struct {
	ID        *string `json:"id,omitempty"`
	ModelID   *string `json:"model_id,omitempty"`
	Timestamp *string `json:"timestamp,omitempty"`
}

// VideoResult is the result of a video generation call.
type VideoResult struct {
	Videos           []VideoData     `json:"videos"`
	Warnings         []Warning       `json:"warnings,omitempty"`
	ProviderMetadata json.RawMessage `json:"provider_metadata,omitempty"`
	Response         VideoResponse   `json:"response"`
}

// VideoCallOptions is the options for video generation.
type VideoCallOptions struct {
	Prompt          *string           `json:"prompt,omitempty"`
	N               *int              `json:"n,omitempty"`
	AspectRatio     *string           `json:"aspect_ratio,omitempty"`
	Resolution      *string           `json:"resolution,omitempty"`
	Seed            *uint64           `json:"seed,omitempty"`
	ProviderOptions jsonObj           `json:"provider_options"`
	Headers         map[string]string `json:"headers,omitempty"`
}

// ── Search ──────────────────────────────────────────────────────────────────

// SearchResultItem is a single search result.
type SearchResultItem struct {
	Title      *string  `json:"title,omitempty"`
	URL        *string  `json:"url,omitempty"`
	Content    *string  `json:"content,omitempty"`
	RawContent *string  `json:"raw_content,omitempty"`
	Score      *float64 `json:"score,omitempty"`
}

// SearchResponse is provider response metadata for search.
type SearchResponse struct {
	ID        *string `json:"id,omitempty"`
	ModelID   *string `json:"model_id,omitempty"`
	Timestamp *string `json:"timestamp,omitempty"`
}

// SearchResult is the result of a search call.
type SearchResult struct {
	Results          []SearchResultItem `json:"results"`
	Answer           *string            `json:"answer,omitempty"`
	ProviderMetadata json.RawMessage    `json:"provider_metadata,omitempty"`
	Warnings         []Warning          `json:"warnings,omitempty"`
	Response         *SearchResponse    `json:"response,omitempty"`
}

// SearchCallOptions is the options for a search call.
type SearchCallOptions struct {
	Query             string            `json:"query"`
	MaxResults        *int              `json:"max_results,omitempty"`
	IncludeRawContent *bool             `json:"include_raw_content,omitempty"`
	TimeRange         *string           `json:"time_range,omitempty"`
	IncludeDomains    []string          `json:"include_domains,omitempty"`
	ExcludeDomains    []string          `json:"exclude_domains,omitempty"`
	ProviderOptions   jsonObj           `json:"provider_options"`
	Headers           map[string]string `json:"headers,omitempty"`
}

// ── Files ───────────────────────────────────────────────────────────────────

// UploadFileResult is the result of a file upload.
type UploadFileResult struct {
	ProviderReference map[string]string `json:"provider_reference"`
	MediaType         *string           `json:"media_type,omitempty"`
	Filename          *string           `json:"filename,omitempty"`
	ProviderMetadata  json.RawMessage   `json:"provider_metadata,omitempty"`
	Warnings          []Warning         `json:"warnings,omitempty"`
}

// UploadFileCallOptions is the options for a file upload.
type UploadFileCallOptions struct {
	Data            json.RawMessage `json:"data"`
	MediaType       string          `json:"media_type"`
	Filename        *string         `json:"filename,omitempty"`
	ProviderOptions jsonObj         `json:"provider_options"`
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
