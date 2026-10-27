//! Amazon Bedrock provider.
//!
//! [`create_amazon_bedrock`] is the Rust form of the AI SDK's
//! `createAmazonBedrock`: it takes [`AmazonBedrockProviderSettings`], validates
//! an explicit base URL and returns an [`AmazonBedrockProvider`]. Nothing is
//! read from the environment and no region, key or credential is needed until
//! a request is made. [`amazon_bedrock()`] is the default instance.
//!
//! The models speak the Bedrock Converse API
//! (`bedrock-runtime.{region}.amazonaws.com/model/{model-id}/converse`), which
//! gives one interface to every Bedrock-hosted model family (Anthropic Claude,
//! Meta Llama, Mistral, Amazon Nova, ...), plus `invoke` for embeddings and
//! images and the Agent Runtime `rerank` endpoint. Claude's native Messages
//! protocol on AWS is a different package ([`crate::anthropic_aws`]) and the
//! OpenAI-compatible Mantle endpoint is a preset; neither is served from here.
//!
//! Authentication is decided for each request, the way `createAmazonBedrock`
//! decides it once at creation:
//!
//! - **API key** when one is available (the `api_key` setting, else
//!   `AWS_BEARER_TOKEN_BEDROCK`): `Authorization: Bearer <key>`, nothing is
//!   signed.
//! - **AWS SigV4** otherwise: a
//!   [`SigV4Fetch`](aimux_provider_utils::SigV4Fetch) transport decorator
//!   signs the final method, URL, headers and body bytes. The credentials come from
//!   `credential_provider`, else `access_key_id` + `secret_access_key` (+
//!   `session_token`), else `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and
//!   `AWS_SESSION_TOKEN`. A missing region or credential fails the request
//!   with `AiMuxError::LoadSetting`.

mod anthropic_model_support;
mod api_types;
mod chat_language_model;
mod chat_language_model_options;
mod convert_to_chat_messages;
mod convert_usage;
mod embedding_model;
mod embedding_model_options;
mod error;
pub mod event_stream_decoder;
mod event_stream_response_handler;
mod image_model;
mod map_finish_reason;
mod normalize_tool_call_id;
pub(crate) mod options;
mod prepare_tools;
mod provider;
pub mod reranking;
mod sigv4_fetch;

pub use chat_language_model::BedrockModel;
pub use chat_language_model_options::{
    AmazonBedrockChatModelFamily, AmazonBedrockChatModelSettings,
};
pub use embedding_model::BedrockEmbeddingModel;
pub use embedding_model_options::{
    AmazonBedrockEmbeddingModelFamily, AmazonBedrockEmbeddingModelSettings,
};
pub use image_model::BedrockImageModel;
pub(crate) use provider::BEARER_TOKEN_ENV_VAR;
pub use provider::{
    AmazonBedrockProvider, AmazonBedrockProviderSettings, amazon_bedrock, create_amazon_bedrock,
};
pub use reranking::BedrockRerankingModel;

#[cfg(test)]
mod tests;
