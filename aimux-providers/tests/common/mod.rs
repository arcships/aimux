//! Shared test infrastructure for `aimux-providers` integration tests.
//!
//! Each `*.rs` file directly under `tests/` is compiled as its own test
//! binary; this module is the conventional place for helpers shared across
//! those binaries. Declare it from a test file with `mod common;` and reach the
//! replay helpers via [`replay`].

pub mod replay;
