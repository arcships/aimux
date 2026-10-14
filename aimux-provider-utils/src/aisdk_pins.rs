//! The `ai-sdk-provider-utils/<v>` part the request helpers append to every
//! request's user-agent, as the AI SDK's `postToApi` / `getFromApi` do.
//!
//! Upstream, `<v>` is the provider-utils that the calling package depends on,
//! so it differs between packages (5.0.51 for `@ai-sdk/groq@4.0.52`, 5.0.53
//! for `@ai-sdk/openai@4.0.83`, ...). The pinned packages in `aimux-providers`
//! name their own version through their transport (`shared::aisdk_pins` and
//! `shared::package_fetch` there); this value is what a request of any other
//! package carries. It is deliberately not the `@ai-sdk/provider-utils` pin of
//! `fixtures/aisdk/VERSIONS.json`, which is the version this crate's port
//! follows and may move without changing any user-agent.

pub(crate) const PROVIDER_UTILS: &str = "5.0.53";
