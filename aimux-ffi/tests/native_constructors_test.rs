//! Smoke tests for the native-protocol C ABI constructors added in 0.2.0
//! (cohere / mistral / xai / bedrock / vertex / anthropic_aws / azure).
//!
//! Constructing a model only builds a config + provider — no network is
//! touched, so fake keys are fine. Constructors return a JSON string
//! (`{"handle":<u64>}` on success, `{"error":...}` on failure).

use std::ffi::CString;
use std::os::raw::c_char;

use aimux_ffi::{
    aimux_anthropic_aws_new, aimux_anthropic_aws_new_with_base, aimux_azure_new,
    aimux_azure_new_with_base, aimux_bedrock_new, aimux_bedrock_new_with_base, aimux_cohere_new,
    aimux_cohere_new_with_base, aimux_drop_handle, aimux_free_string, aimux_mistral_new,
    aimux_mistral_new_with_base, aimux_vertex_new, aimux_vertex_new_with_base, aimux_xai_new,
    aimux_xai_new_with_base,
};

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

/// Copy a constructor's JSON result into an owned `String`, freeing the
/// native pointer.
fn take_json(json_ptr: *mut c_char, name: &str) -> String {
    assert!(!json_ptr.is_null(), "{name}: null result pointer");
    let json = unsafe { std::ffi::CStr::from_ptr(json_ptr) }
        .to_str()
        .unwrap()
        .to_string();
    unsafe { aimux_free_string(json_ptr) };
    json
}

/// Assert a constructor's JSON result is `{"handle":<u64>}` (not an error
/// envelope) and release the handle.
fn expect_handle(json_ptr: *mut c_char, name: &str) {
    let json = take_json(json_ptr, name);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap_or_else(|e| {
        panic!("{name}: result is not valid JSON ({e}): {json}");
    });
    assert!(
        value.get("error").is_none(),
        "{name}: expected success, got error envelope: {json}"
    );
    let handle = value
        .get("handle")
        .and_then(|h| h.as_u64())
        .unwrap_or_else(|| panic!("{name}: expected {{\"handle\":<u64>}}, got {json}"));
    assert!(handle != 0, "{name}: handle should never be 0 on success");
    aimux_drop_handle(handle);
}

/// Assert a constructor's JSON result is an error envelope
/// (`{"error":...}`), not a handle.
fn expect_error(json_ptr: *mut c_char, name: &str) {
    let json = take_json(json_ptr, name);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap_or_else(|e| {
        panic!("{name}: result is not valid JSON ({e}): {json}");
    });
    assert!(
        value.get("error").is_some(),
        "{name}: expected error envelope, got {json}"
    );
}

#[test]
fn simple_key_constructors() {
    let key = c("sk-test-fake-key");
    let model = c("command-r-plus");
    expect_handle(aimux_cohere_new(key.as_ptr(), model.as_ptr()), "cohere_new");
    let base = c("https://example.com/v1");
    expect_handle(
        aimux_cohere_new_with_base(key.as_ptr(), model.as_ptr(), base.as_ptr()),
        "cohere_new_with_base",
    );

    let model = c("mistral-large-latest");
    expect_handle(
        aimux_mistral_new(key.as_ptr(), model.as_ptr()),
        "mistral_new",
    );
    expect_handle(
        aimux_mistral_new_with_base(key.as_ptr(), model.as_ptr(), base.as_ptr()),
        "mistral_new_with_base",
    );

    let model = c("grok-3");
    expect_handle(aimux_xai_new(key.as_ptr(), model.as_ptr()), "xai_new");
    expect_handle(
        aimux_xai_new_with_base(key.as_ptr(), model.as_ptr(), base.as_ptr()),
        "xai_new_with_base",
    );
}

#[test]
fn credential_constructors() {
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
        ),
        "vertex_new",
    );
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

    let key = c("sk-ant-fake");
    let model = c("claude-3-5-sonnet-20240620-v1:0");
    expect_handle(
        aimux_anthropic_aws_new(key.as_ptr(), region.as_ptr(), model.as_ptr()),
        "anthropic_aws_new",
    );
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

#[test]
fn azure_constructors() {
    let key = c("sk-azure-fake");
    let resource = c("my-resource");
    let deployment = c("gpt-4o");
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

#[test]
fn invalid_args_return_error() {
    // Null model_id must fail cleanly (error envelope, no panic).
    let key = c("sk-test-fake-key");
    expect_error(
        aimux_cohere_new(key.as_ptr(), std::ptr::null()),
        "cohere_new(null model_id)",
    );
    expect_error(
        aimux_bedrock_new(key.as_ptr(), key.as_ptr(), key.as_ptr(), std::ptr::null()),
        "bedrock_new(null model_id)",
    );
}
