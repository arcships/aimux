//! # Azure OpenAI provider
//!
//! Implements the Azure OpenAI chat-completions API. Azure speaks the OpenAI
//! wire format but uses a deployment-based URL with an `api-version` query
//! parameter and authenticates via either an `api-key` header or an Azure AD
//! (Microsoft Entra ID) bearer token.
//!
//! See [`AzureProvider`] and [`AzureConfig`](model::AzureConfig).

pub mod model;
pub mod responses;

pub use model::{
    AzureAuth, AzureConfig, AzureModel, AzureProvider, DEFAULT_API_VERSION, TokenProvider,
};
pub use responses::AzureResponsesModel;
