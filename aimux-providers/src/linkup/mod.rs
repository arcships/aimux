//! Linkup provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Linkup search API
//! (`POST https://api.linkup.so/v1/search`).
//!
//! Linkup is a modality-specific provider: it exposes a native web search
//! protocol (query → results) and does not support language models. The
//! `depth` and `outputType` request fields are passed through via
//! `provider_options` under the `"linkup"` key.
//!
//! [`create_linkup`] takes [`LinkupProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`LinkupProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `LINKUP_API_KEY`.
//! [`linkup()`] is the default instance; it reads nothing and cannot fail.

mod provider;
mod search_model;

pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{LinkupProvider, LinkupProviderSettings, create_linkup, linkup};
pub use search_model::LinkupSearchModel;
