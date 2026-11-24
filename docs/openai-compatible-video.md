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
own origin is downloaded by the provider, and the form of the reference
decides whether the gateway's configured query parameters (`api-version`,
`key`) and credentials are added. A relative reference addresses the gateway's
API: it resolves against the base URL as a directory, must remain on that
origin, and gets the configured parameters appended after the query bytes it
carries; the received pairs keep their order and encoding, and a parameter the
reference already carries keeps the value the gateway sent. An absolute URL is
a complete target: the gateway named the whole address and chose its query, so
it is sent exactly as received — its path and query bytes are neither changed
nor re-encoded — and the configured parameters and credentials are not added,
because the query may be a signature or token over its canonical form and a
provider credential next to it would be a second authentication scheme. An
absolute URL without a query cannot carry a query signature, so it is treated
as an asset route and gets the configured parameters and credentials. This is
a rule about the shape of the URL, not about parameter names: an unfamiliar
signature scheme in an absolute URL is preserved without being recognized. A
relative reference whose own query authenticates it — a presigned link such as
SigV4 `X-Amz-*`, `X-Goog-*`, `Signature`, `sig`, a token, or an Akamai
`hdnts`, `hmac`, `jwt`, `credential` or `auth` parameter — is likewise only
resolved, with no configured parameters and no credential; that name list is a
supplement to the structural rule and is consulted only for the relative form.
If `url` is absent/null, the provider downloads `/videos/{id}/content`, with
the configured parameters and credentials. A gateway that needs its query
parameters on an absolute asset URL includes them in the URL it returns.
Download requests use the existing SSRF guard, allow the explicitly configured
origin, and strip caller headers on a redirect to another origin. Non-HTTP
schemes, foreign protocol-relative references, and URL userinfo are rejected.
Creation and content-download exchanges move whole media files, so they use a
600-second exchange guard instead of the 30-second default.

A creation the gateway refused is not replayed, because a timeout, 5xx or
malformed successful response may follow an accepted, billable task; a
rate-limited creation (HTTP 429) queued no task and stays retryable. Status
failures keep the usual retry classification. An authenticated content
download retries a transient failure a bounded number of times and is
non-retryable once those attempts are exhausted; retrying the status or
download phase never creates another task. No cancellation endpoint is
provided.
