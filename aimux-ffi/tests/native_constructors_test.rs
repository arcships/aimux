//! Smoke tests for the native-protocol C ABI constructors added in 0.2.0
//! (cohere / mistral / xai / bedrock / vertex / anthropic_aws / azure).
//!
//! Constructing a model only builds a config + provider — no network is
//! touched, so fake keys are fine. A non-zero handle means the constructor
//! wired the provider correctly.

use std::ffi::CString;

use aimux_ffi::{
    aimux_anthropic_aws_new, aimux_anthropic_aws_new_with_base, aimux_azure_new,
    aimux_azure_new_with_base, aimux_bedrock_new, aimux_bedrock_new_with_base, aimux_cohere_new,
    aimux_cohere_new_with_base, aimux_drop_handle, aimux_mistral_new, aimux_mistral_new_with_base,
    aimux_vertex_new, aimux_vertex_new_with_base, aimux_xai_new, aimux_xai_new_with_base,
};

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

fn expect_handle(h: u64, name: &str) {
    assert!(h != 0, "{name}: expected non-zero handle");
    unsafe { aimux_drop_handle(h) };
}

#[test]
fn simple_key_constructors() {
    let key = c("sk-test-fake-key");
    let model = c("command-r-plus");
    unsafe {
        expect_handle(aimux_cohere_new(key.as_ptr(), model.as_ptr()), "cohere_new");
        let base = c("https://example.com/v1");
        expect_handle(
            aimux_cohere_new_with_base(key.as_ptr(), model.as_ptr(), base.as_ptr()),
            "cohere_new_with_base",
        );
    }
    let model = c("mistral-large-latest");
    unsafe {
        expect_handle(
            aimux_mistral_new(key.as_ptr(), model.as_ptr()),
            "mistral_new",
        );
        let base = c("https://example.com/v1");
        expect_handle(
            aimux_mistral_new_with_base(key.as_ptr(), model.as_ptr(), base.as_ptr()),
            "mistral_new_with_base",
        );
    }
    let model = c("grok-3");
    unsafe {
        expect_handle(aimux_xai_new(key.as_ptr(), model.as_ptr()), "xai_new");
        let base = c("https://example.com/v1");
        expect_handle(
            aimux_xai_new_with_base(key.as_ptr(), model.as_ptr(), base.as_ptr()),
            "xai_new_with_base",
        );
    }
}

#[test]
fn credential_constructors() {
    let access = c("AKIDEXAMPLE");
    let secret = c("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY");
    let region = c("us-east-1");
    let model = c("anthropic.claude-3-5-sonnet-20240620-v1:0");
    unsafe {
        expect_handle(
            aimux_bedrock_new(
                access.as_ptr(),
                secret.as_ptr(),
                region.as_ptr(),
                model.as_ptr(),
            ),
            "bedrock_new",
        );
        let base = c("https://example.com");
        expect_handle(
            aimux_bedrock_new_with_base(
                access.as_ptr(),
                secret.as_ptr(),
                region.as_ptr(),
                model.as_ptr(),
                base.as_ptr(),
            ),
            "bedrock_new_with_base",
        );
    }

    let token = c("ya29.fake-token");
    let project = c("my-gcp-project");
    let location = c("us-central1");
    let model = c("gemini-2.0-flash");
    unsafe {
        expect_handle(
            aimux_vertex_new(
                token.as_ptr(),
                project.as_ptr(),
                location.as_ptr(),
                model.as_ptr(),
            ),
            "vertex_new",
        );
        let base = c("https://example.com");
        expect_handle(
            aimux_vertex_new_with_base(
                token.as_ptr(),
                project.as_ptr(),
                location.as_ptr(),
                model.as_ptr(),
                base.as_ptr(),
            ),
            "vertex_new_with_base",
        );
    }

    let key = c("sk-ant-fake");
    let region = c("us-east-1");
    let model = c("claude-3-5-sonnet-20240620-v1:0");
    unsafe {
        expect_handle(
            aimux_anthropic_aws_new(key.as_ptr(), region.as_ptr(), model.as_ptr()),
            "anthropic_aws_new",
        );
        let base = c("https://example.com");
        expect_handle(
            aimux_anthropic_aws_new_with_base(
                key.as_ptr(),
                region.as_ptr(),
                model.as_ptr(),
                base.as_ptr(),
            ),
            "anthropic_aws_new_with_base",
        );
    }
}

#[test]
fn azure_constructors() {
    let key = c("sk-azure-fake");
    let resource = c("my-resource");
    let deployment = c("gpt-4o");
    unsafe {
        expect_handle(
            aimux_azure_new(
                key.as_ptr(),
                resource.as_ptr(),
                deployment.as_ptr(),
                std::ptr::null(),
            ),
            "azure_new (default api_version)",
        );
        let version = c("2024-06-01");
        expect_handle(
            aimux_azure_new(
                key.as_ptr(),
                resource.as_ptr(),
                deployment.as_ptr(),
                version.as_ptr(),
            ),
            "azure_new (explicit api_version)",
        );
        let base = c("https://example.openai.azure.com");
        expect_handle(
            aimux_azure_new_with_base(
                key.as_ptr(),
                base.as_ptr(),
                deployment.as_ptr(),
                std::ptr::null(),
            ),
            "azure_new_with_base",
        );
    }
}

#[test]
fn invalid_args_return_zero() {
    // Null model_id must fail cleanly (returns 0, no panic).
    let key = c("sk-test-fake-key");
    unsafe {
        assert_eq!(aimux_cohere_new(key.as_ptr(), std::ptr::null()), 0);
        assert_eq!(
            aimux_bedrock_new(key.as_ptr(), key.as_ptr(), key.as_ptr(), std::ptr::null()),
            0
        );
    }
}
