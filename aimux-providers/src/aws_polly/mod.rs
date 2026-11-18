//! Amazon Polly speech (TTS) provider.
//!
//! Implements the `SpeechModel` trait against the Amazon Polly `SynthesizeSpeech`
//! API (`POST https://polly.{region}.amazonaws.com/v1/speech`).
//!
//! Authentication uses AWS Signature Version 4 (SigV4), applied by the shared
//! [`SigV4Fetch`](aimux_provider_utils::SigV4Fetch) transport decorator over the
//! final request bytes (the same decorator Bedrock uses). The service name is
//! `"polly"`.
//!
//! The Polly API accepts a JSON request body describing the synthesis job and
//! returns the synthesized audio as a binary stream (the response body is *not*
//! JSON). The `Content-Type` of the response reflects the requested
//! `OutputFormat`.
//!
//! Environment variables follow the standard AWS credential chain:
//! `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_REGION` (defaulting to
//! `us-east-1`), and the optional `AWS_SESSION_TOKEN` for temporary STS
//! credentials.
//!
//! [`create_aws_polly`] takes [`AwsPollyProviderSettings`], validates the base URL, fixes
//! the provider name and returns a [`AwsPollyProvider`]. The credentials are not
//! read there: they are loaded for every request, from the settings or from the
//! AWS environment variables.
//! [`aws_polly()`] is the default instance; it reads nothing and cannot fail.

mod provider;
mod speech_model;

pub use provider::{AwsPollyProvider, AwsPollyProviderSettings, aws_polly, create_aws_polly};
pub use speech_model::AwsPollySpeechModel;
