# ByteDance Ark media

The built-in `bytedance` provider now exposes native Seedance video and Seedream image models alongside its existing OpenAI-compatible chat, embeddings, and discovery. The standalone Rust factory is `create_bytedance(ByteDanceProviderSettings)`; its `image(id)` and `video(id)` methods accept model IDs, Ark endpoint IDs, and relay aliases without a model-name allowlist.

Credentials use Bearer authentication and default to `ARK_API_KEY`. The default base URL is `https://ark.cn-beijing.volces.com/api/v3`. Custom HTTP(S) bases retain their path prefix and may point at an Ark-compatible relay. Factory and unified settings preserve custom headers, transport, and request-body transforms. The `byteplus` text preset is unchanged.

```rust,ignore
use aimux_providers::{create_provider, preset::PresetSettings};
use aimux_core::image_model::{ImageCallOptions, generate_image};

let provider = create_provider("bytedance", PresetSettings::default())?;
let model = provider.image_model("doubao-seedream-4-0-250828")?;
let result = generate_image(model.as_ref(), ImageCallOptions::new("A watercolor cat")).await?;
```

## Seedance tasks

`do_start` sends `POST /contents/generations/tasks` and returns an opaque JSON operation containing `id`. `do_status` sends `GET /contents/generations/tasks/{id}`. `queued` and `running` remain pending; `succeeded` returns `content.video_url`. Optional `content.last_frame_url` is available as `providerMetadata.bytedance.lastFrameUrl`. `failed`, `cancelled`, and `expired` surface `error.message` with the task status as `providerCode`. All three are terminal and non-retryable: an expired task reaches the host as `providerCode: expired`, so the host decides whether to submit a new task, and the status query is never replayed.

A text prompt becomes a `text` content element. `image` is the first frame; `frame_images` maps first/last frame roles. `input_references` maps images (bytes, base64, or URL) to `reference_image`, and explicitly typed video URLs to `reference_video`. Inline image data is serialized as a data URI. Local video upload is unsupported. Reference inputs and first/last frames cannot be mixed; duplicate frames and a last frame without a first frame fail before network access. Media-only requests are supported.

The `bytedance` provider-options namespace accepts:

| Option | Wire behavior |
| --- | --- |
| `resolution`, `ratio` | Ark string parameters, e.g. `720p`, `16:9` |
| `duration`, `frames`, `framespersecond`, `seed` | Integer parameters |
| `generate_audio`, `return_last_frame` | Boolean parameters |
| `service_tier`, `execution_expires_after`, `callback_url` | Task scheduling and callback parameters |
| `imageUrls`, `videoUrls`, `audioUrls` | HTTP(S) URL arrays converted to corresponding content objects and reference roles |

Common duration, fps, seed, aspect ratio, and generate-audio options are mapped when the corresponding provider option is absent. Pixel-based `resolution` is rejected: use Ark's provider-specific resolution string. A task produces one video. Unknown options and incorrect types fail locally; model-specific availability and limits remain the server's responsibility.

## Seedream images

Generation and editing both use JSON `POST /images/generations`. `files` is serialized as `image`: one URL/data URI for a single reference, an array for multiple references. No multipart edits request is sent. Masks are unsupported. `size` maps to Ark's width-by-height string; `providerOptions.bytedance.size` also accepts resolution tiers such as `2K`. Aspect ratio emits an unsupported-option warning; use size instead.

Options in `bytedance` are `watermark`, `background`, `sequential_image_generation`, `sequential_image_generation_options`, `optimize_prompt_options`, `guidance_scale`, `stream`, `response_format`, and `size`. `response_format` is left to Ark's `url` default unless set, so a sequential group cannot exhaust the JSON response budget with base64; `b64_json` remains available for callers that want it. `stream: true` consumes Ark's image-generation SSE events and waits for `image_generation.completed`; malformed, failed, and incomplete streams return errors. Available modes vary by model. Sequential generation may return multiple related images even though a single call has `n = 1`; use the native sequential options for that behavior.

Both formats produce binary image outputs. URL downloads carry no Ark credentials or configured provider headers and pass through the shared download URL guard. Image metadata retains fields such as size and image index. Token usage is mapped when present. The image request has a 600-second response timeout; Core owns the overall call deadline and cancellation.

Generation POST errors are non-retryable except HTTP 429: a rate-limited request queued no task or image, so replaying it cannot bill twice. Asset downloads use the shared bounded retry (up to three attempts) and keep Ark credentials off the request; once those attempts are spent the error is non-retryable, so a billed generation is never replayed. Status GET requests retain normal retry behavior.

## Protocol sources and verification

Protocol schemas checked against official documentation on 2026-11-20:

- [Create video task](https://docs.volcengine.com/docs/ark/create-video-generation-task-api?lang=zh)
- [Get video task](https://docs.volcengine.com/docs/ark/get-video-generation-task-api?lang=zh)
- [Image generation](https://docs.volcengine.com/docs/ark/image-generation-api?lang=zh)
- [Image streaming events](https://docs.volcengine.com/docs/ark/image-generation-streaming-responses?lang=zh)

`aimux-providers/tests/bytedance_media_test.rs` uses local wiremock fixtures derived from those schemas for requests, response formats, task states, errors, input validation, relay prefixes, credential-free downloads, and unified factory composition. These are synthetic contract fixtures, not recorded live cassettes. No real API call or billable generation was performed.
