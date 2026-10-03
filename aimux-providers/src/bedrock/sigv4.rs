//! AWS Signature Version 4 signing for Bedrock and Claude Platform on AWS.
//!
//! The signer lives in `aimux-provider-utils` (`sigv4_fetch`), where it also
//! backs the `SigV4Fetch` transport decorator. The models here still call
//! `sign_request` directly; they move to the decorator in A4.

pub use aimux_provider_utils::sigv4_fetch::{AwsCredentials, SignedRequest, sign_request};
