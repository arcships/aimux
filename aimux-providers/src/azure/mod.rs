//! # Azure OpenAI provider
//!
//! [`create_azure`] is the Rust form of the AI SDK's `createAzure`: it takes
//! [`AzureOpenAIProviderSettings`], rejects a non-empty `api_key` given with a
//! `token_provider`, validates the base URL and returns an
//! [`AzureOpenAIProvider`]. Nothing is read from the environment there: the
//! API key (`AZURE_API_KEY`), the Entra ID token and the resource name
//! (`AZURE_RESOURCE_NAME`) are resolved on every request, so a missing one
//! fails that request with `AiMuxError::LoadApiKey` / `LoadSetting`.
//! [`azure()`] is the default instance.
//!
//! Azure speaks the OpenAI wire format, so its models are the OpenAI package's
//! (`crate::openai`) configured for Azure: provider strings
//! `azure.chat` / `azure.responses` / `azure.embeddings` / `azure.image` /
//! `azure.transcription` / `azure.speech`, a URL that follows the AI SDK's rules
//! (`{base}/v1{path}?api-version=` or the deployment form
//! `{base}/deployments/{id}{path}?api-version=`), `api-key` or
//! `Authorization: Bearer` authentication, and the Responses model's `azure`
//! providerOptions namespace and `assistant-` file-id prefix.
//!
//! Not ported from the AI SDK package: Azure-hosted DeepSeek (`deepseek`),
//! the legacy completion model and MAI-Voice (`maiBaseURL`, `webSocket`).

mod openai_provider;
mod openai_tools;
pub(crate) mod options;
mod speech_transcription_model;
mod speech_transcription_model_options;
mod transcription_model_options;

pub(crate) use openai_provider::API_KEY_ENV_VAR;
pub use openai_provider::{
    AzureChatModel, AzureOpenAIProvider, AzureOpenAIProviderSettings, AzureResponsesModel,
    AzureTranscriptionModel, azure, create_azure,
};
pub use openai_tools::AzureOpenAITools;

#[cfg(test)]
mod tests;
