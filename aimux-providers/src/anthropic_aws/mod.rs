//! Anthropic-AWS (Claude Platform on AWS) provider.
//!
//! The Claude Platform on AWS API (`aws-external-anthropic.{region}.api.aws/v1`)
//! is the Anthropic Messages API hosted in AWS, so [`create_anthropic_aws`]
//! returns a provider whose models are the shared
//! [`AnthropicMessagesModel`](crate::anthropic::AnthropicMessagesModel). Only
//! the endpoint and the authentication differ: an AWS-provisioned API key, or
//! AWS SigV4 applied by a [`SigV4Fetch`](aimux_provider_utils::SigV4Fetch)
//! transport decorator over the final request.
//!
//! The settings follow `@ai-sdk/anthropic-aws` 2.0.63: the region comes from
//! the setting or `AWS_REGION` (no default), the workspace id from the setting
//! or `ANTHROPIC_AWS_WORKSPACE_ID` (required), and with no `auth` a request
//! uses `ANTHROPIC_AWS_API_KEY` when it is set and SigV4 with
//! `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / `AWS_SESSION_TOKEN`
//! otherwise. All of them are read when a request is made.
//!
//! Reference: <https://docs.anthropic.com/en/api/messages>

mod fetch;
mod provider;

pub(crate) use fetch::{Auth, AwsHeaders};
pub use provider::{
    AnthropicAwsAuth, AnthropicAwsModel, AnthropicAwsProvider, AnthropicAwsProviderSettings,
    anthropic_aws, create_anthropic_aws,
};

pub use crate::shared::TransformRequestBody;

#[cfg(test)]
mod tests;
