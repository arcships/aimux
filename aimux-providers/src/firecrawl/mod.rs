//! Firecrawl provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Firecrawl search API
//! (`POST https://api.firecrawl.dev/v2/search`). Bearer auth via
//! `FIRECRAWL_API_KEY`.
//!
//! [`create_firecrawl`] takes [`FirecrawlProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`FirecrawlProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `FIRECRAWL_API_KEY`.
//! [`firecrawl()`] is the default instance; it reads nothing and cannot fail.

mod provider;
mod search_model;

pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{FirecrawlProvider, FirecrawlProviderSettings, create_firecrawl, firecrawl};
pub use search_model::FirecrawlSearchModel;
