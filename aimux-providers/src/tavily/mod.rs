//! Tavily provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Tavily search API
//! (`POST https://api.tavily.com/search`). Bearer auth via `TAVILY_API_KEY`.
//!
//! [`create_tavily`] takes [`TavilyProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`TavilyProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `TAVILY_API_KEY`.
//! [`tavily()`] is the default instance; it reads nothing and cannot fail.

mod provider;
mod search_model;

pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{TavilyProvider, TavilyProviderSettings, create_tavily, tavily};
pub use search_model::TavilySearchModel;
