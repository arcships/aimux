//! Smoke tests for the native-protocol C ABI constructors
//! (cohere / mistral / xai / bedrock / vertex / anthropic_aws / azure).
//!
//! Constructing a model only builds a config + provider — no network is
//! touched, so fake keys are fine. Constructors return NULL and write a
//! non-zero handle to `*out_handle`, or return an `aimux_error_t`.

mod common;

use std::ptr;

use aimux_ffi::{
    aimux_anthropic_aws_new, aimux_anthropic_aws_new_with_base, aimux_azure_new,
    aimux_azure_new_with_base, aimux_bedrock_new, aimux_bedrock_new_with_base, aimux_cohere_new,
    aimux_cohere_new_with_base, aimux_drop_handle, aimux_error_t, aimux_mistral_new,
    aimux_mistral_new_with_base, aimux_vertex_new, aimux_vertex_new_with_base, aimux_xai_new,
    aimux_xai_new_with_base,
};
use common::{c, expect_ffi_error, ok};

fn expect_handle(e: *mut aimux_error_t, h: u64, name: &str) {
    ok(e, name);
    assert_ne!(h, 0, "{name}: expected non-zero handle");
    aimux_drop_handle(h);
}

#[test]
fn simple_key_constructors() {
    let key = c("sk-test-fake-key");
    let model = c("command-r-plus");
    let base = c("https://example.com/v1");
    let mut h = 0;
    expect_handle(
        aimux_cohere_new(key.as_ptr(), model.as_ptr(), &mut h),
        h,
        "cohere_new",
    );
    expect_handle(
        aimux_cohere_new_with_base(key.as_ptr(), model.as_ptr(), base.as_ptr(), &mut h),
        h,
        "cohere_new_with_base",
    );

    let model = c("mistral-large-latest");
    expect_handle(
        aimux_mistral_new(key.as_ptr(), model.as_ptr(), &mut h),
        h,
        "mistral_new",
    );
    expect_handle(
        aimux_mistral_new_with_base(key.as_ptr(), model.as_ptr(), base.as_ptr(), &mut h),
        h,
        "mistral_new_with_base",
    );

    let model = c("grok-3");
    expect_handle(
        aimux_xai_new(key.as_ptr(), model.as_ptr(), &mut h),
        h,
        "xai_new",
    );
    expect_handle(
        aimux_xai_new_with_base(key.as_ptr(), model.as_ptr(), base.as_ptr(), &mut h),
        h,
        "xai_new_with_base",
    );
}

#[test]
fn credential_constructors() {
    let mut h = 0;
    let access = c("AKIDEXAMPLE");
    let secret = c("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY");
    let region = c("us-east-1");
    let model = c("anthropic.claude-3-5-sonnet-20240620-v1:0");
    expect_handle(
        aimux_bedrock_new(
            access.as_ptr(),
            secret.as_ptr(),
            region.as_ptr(),
            model.as_ptr(),
            &mut h,
        ),
        h,
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
            &mut h,
        ),
        h,
        "bedrock_new_with_base",
    );

    let token = c("ya29.fake-token");
    let project = c("my-gcp-project");
    let location = c("us-central1");
    let model = c("gemini-2.0-flash");
    expect_handle(
        aimux_vertex_new(
            token.as_ptr(),
            project.as_ptr(),
            location.as_ptr(),
            model.as_ptr(),
            &mut h,
        ),
        h,
        "vertex_new",
    );
    expect_handle(
        aimux_vertex_new_with_base(
            token.as_ptr(),
            project.as_ptr(),
            location.as_ptr(),
            model.as_ptr(),
            base.as_ptr(),
            &mut h,
        ),
        h,
        "vertex_new_with_base",
    );

    let key = c("sk-ant-fake");
    let model = c("claude-3-5-sonnet-20240620-v1:0");
    expect_handle(
        aimux_anthropic_aws_new(key.as_ptr(), region.as_ptr(), model.as_ptr(), &mut h),
        h,
        "anthropic_aws_new",
    );
    expect_handle(
        aimux_anthropic_aws_new_with_base(
            key.as_ptr(),
            region.as_ptr(),
            model.as_ptr(),
            base.as_ptr(),
            &mut h,
        ),
        h,
        "anthropic_aws_new_with_base",
    );
}

#[test]
fn azure_constructors() {
    let mut h = 0;
    let key = c("sk-azure-fake");
    let resource = c("my-resource");
    let deployment = c("gpt-4o");
    expect_handle(
        aimux_azure_new(
            key.as_ptr(),
            resource.as_ptr(),
            deployment.as_ptr(),
            ptr::null(),
            &mut h,
        ),
        h,
        "azure_new (default api_version)",
    );
    let version = c("2024-06-01");
    expect_handle(
        aimux_azure_new(
            key.as_ptr(),
            resource.as_ptr(),
            deployment.as_ptr(),
            version.as_ptr(),
            &mut h,
        ),
        h,
        "azure_new (explicit api_version)",
    );
    let base = c("https://example.openai.azure.com");
    expect_handle(
        aimux_azure_new_with_base(
            key.as_ptr(),
            base.as_ptr(),
            deployment.as_ptr(),
            ptr::null(),
            &mut h,
        ),
        h,
        "azure_new_with_base",
    );
}

#[test]
fn invalid_args_return_error() {
    let mut h = 7;
    let key = c("sk-test-fake-key");
    let e = aimux_cohere_new(key.as_ptr(), ptr::null(), &mut h);
    assert_eq!(h, 0, "failure writes the sentinel");
    assert_eq!(
        expect_ffi_error(e, "cohere_new(null model_id)"),
        "model_id: must not be NULL"
    );
    let e = aimux_bedrock_new(
        key.as_ptr(),
        key.as_ptr(),
        key.as_ptr(),
        ptr::null(),
        &mut h,
    );
    assert_eq!(h, 0);
    assert_eq!(
        expect_ffi_error(e, "bedrock_new(null model_id)"),
        "model_id: must not be NULL"
    );
}
