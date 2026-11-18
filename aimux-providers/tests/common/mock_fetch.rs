//! Temporary forwarding shim so `tests/fal_image_test.rs` (untouched per
//! open-PR conflict rules) compiles without modifying its `#[path]` include.
#![allow(dead_code, unused_imports)]
pub use aimux_providers::test_support::mock_fetch::*;
