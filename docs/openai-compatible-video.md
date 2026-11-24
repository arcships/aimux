# OpenAI-compatible video gateways

`create_openai_compatible(settings).video(model_id)` and the provider trait's
`video_model(model_id)` expose asynchronous `/videos` gateways (vLLM-Omni,
New API and compatible relays). Registry presets using the compatible provider
also expose this model; the selected endpoint must implement this protocol.
This does not add a video model to the official `openai` package.

Use `aimux_core::video_model::generate_video` for the Core-owned polling loop.
The provider sends `POST {base}/videos` as multipart and polls
`GET {base}/videos/{id}`. The model reports `"{name}.video"` and produces one
video per API call; Core batches a larger requested `n`.

## Request fields

- `prompt` and `model` are sent as text fields.
- `duration` maps to `seconds`; `resolution` maps to `size`.
- `aspect_ratio`, `fps`, `seed`, and `generate_audio` map to the corresponding
  multipart fields when present.
- Provider options merge `openaiCompatible`, the configured provider name, and
  its camelCase spelling (later namespaces win). String values pass unchanged;
  all other JSON values are JSON encoded. Explicit standard call options take
  precedence over the corresponding provider options. `model` and `prompt`
  always come from the selected model and call.
- `image`, `frame_images`, and `input_references` produce file parts from the
  reference inputs: one file is sent in the singular `input_reference` part,
  two or more as repeated `input_references` parts. vLLM-Omni reads one file
  from the singular name and an ordered list from the plural one; repeated
  singular parts collapse to the last file, which loses a first frame.
  Role-tagged frames are ordered first frame, last frame, followed by reference
  inputs. A duplicate first/last frame is rejected before submission. Inline
  binary/base64 files and downloadable URLs are supported. Input URL downloads
  carry no provider credentials and use the SSRF guard.

For MiniMax H3 through vLLM-Omni, put `width`, `height`,
`num_inference_steps`, `flow_shift`, and `extra_params` in provider options.
`extra_params` can be an object or an already encoded JSON string. H3-specific
size, duration, and task constraints belong to the caller or preset; the generic
model does not impose them. Use `frame_images` to provide first and last frames
with repeated `input_references` parts.

## Completion and download

`queued`, `pending`, `processing`, `in_progress`, and `running` mean pending.
`completed` and `succeeded` mean complete. Terminal failures carry the
provider's error message and the terminal status as `providerCode`. A status
the model does not recognize is treated as pending and keeps polling until the
poll timeout, so a gateway-specific alias does not fail a task that is still
running; a returned task ID that does not match the operation is rejected.

An absolute HTTP(S) result URL on another origin becomes `VideoData::Url`;
Core downloads it without provider credentials. A result URL on the provider's
own origin — absolute or relative — is downloaded by the provider: a relative
one resolves against the base URL as a directory and must remain on that
origin, and an absolute one is downloaded only while its origin matches. If
`url` is absent/null, the provider downloads `/videos/{id}/content`. An
unsigned asset URL carries the provider's configured query parameters
(`api-version`, `key`) and the configured credentials. A URL whose own query
authenticates it — a presigned link, such as SigV4 `X-Amz-Signature`, a
`Signature`, `sig` or token parameter — is requested exactly as received: its
path and query are neither modified nor re-encoded, and no provider credential
is attached, because a signature covers the whole canonical query and a second
authentication scheme next to it is an auth conflict. Download requests use
the existing SSRF guard, allow the explicitly configured origin, and strip
caller headers on a redirect to another origin. Non-HTTP schemes, foreign
protocol-relative references, and URL userinfo are rejected. Creation and
content-download exchanges move whole media files, so they use a 600-second
exchange guard instead of the 30-second default.

A creation the gateway refused is not replayed, because a timeout, 5xx or
malformed successful response may follow an accepted, billable task; a
rate-limited creation (HTTP 429) queued no task and stays retryable. Status
failures keep the usual retry classification. An authenticated content
download retries a transient failure a bounded number of times and is
non-retryable once those attempts are exhausted; retrying the status or
download phase never creates another task. No cancellation endpoint is
provided.
