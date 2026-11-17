//! Google Programmable Search Engine (PSE) provider — search modality only.
//!
//! Implements the `SearchModel` trait against the Google Custom Search JSON
//! API (`GET https://www.googleapis.com/customsearch/v1`).
//!
//! Google PSE is a modality-specific provider: it exposes a web search
//! protocol and does not support language models. Authentication uses an API
//! key (`GOOGLE_API_KEY`) and a search-engine ID / `cx` (`GOOGLE_CSE_ID`),
//! both passed as query parameters. The `cx` may also be supplied at call
//! time via `provider_options["google_pse"]["cx"]`.
//!
//! [`create_google_pse`] takes [`GooglePseProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`GooglePseProvider`]. The credential is not read
//! there: it is loaded for every request, from the setting or from `GOOGLE_API_KEY`.
//! [`google_pse()`] is the default instance; it reads nothing and cannot fail.

mod provider;
mod search_model;

pub(crate) use provider::{API_KEY_ENV_VAR, DEFAULT_BASE_URL};
pub use provider::{GooglePseProvider, GooglePseProviderSettings, create_google_pse, google_pse};
pub use search_model::GooglePseSearchModel;
