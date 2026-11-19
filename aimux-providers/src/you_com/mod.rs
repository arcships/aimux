//! You.com search provider — implements the `SearchModel` trait.
//!
//! Implements the You.com (YDC Index) search API
//! (`GET https://ydc-index.io/v1/search?query=...&count=...`).
//!
//! Authentication uses the `X-API-Key` header (env `YDC_API_KEY`). Note the
//! base URL is `ydc-index.io`, not `you.com`. You.com is a search-only
//! provider: it exposes a web search protocol (query → results) and does not
//! support language models.
//!
//! [`create_you_com`] takes [`YouComProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`YouComProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `YDC_API_KEY`.
//! [`you_com()`] is the default instance; it reads nothing and cannot fail.

mod provider;
mod search_model;

pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{YouComProvider, YouComProviderSettings, create_you_com, you_com};
pub use search_model::YouComSearchModel;
