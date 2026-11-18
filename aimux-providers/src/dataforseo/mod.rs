//! DataForSEO search provider — implements the `SearchModel` trait.
//!
//! Implements the DataForSEO SERP Google Organic Live Advanced API
//! (`POST https://api.dataforseo.com/v3/serp/google/organic/live/advanced`).
//!
//! Authentication uses HTTP Basic auth with `DATAFORSEO_LOGIN` and
//! `DATAFORSEO_PASSWORD` credentials. The request body is a JSON array of
//! task objects and the response is deeply nested. DataForSEO is a
//! search-only provider that does not support language models.
//!
//! [`create_dataforseo`] takes [`DataforseoProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`DataforseoProvider`]. The credential is not read
//! there: it is loaded for every request, from the settings or from `DATAFORSEO_LOGIN` and `DATAFORSEO_PASSWORD`.
//! [`dataforseo()`] is the default instance; it reads nothing and cannot fail.

mod provider;
mod search_model;

pub(crate) use provider::DEFAULT_BASE_URL;
pub use provider::{DataforseoProvider, DataforseoProviderSettings, create_dataforseo, dataforseo};
pub use search_model::DataforseoSearchModel;
