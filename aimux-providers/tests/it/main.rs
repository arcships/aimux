//! Provider tests that go through the public API: provider → model → mock
//! fetch, one directory per vendor (RFC-0046 §5).

#[path = "../common/mod.rs"]
mod common;
#[path = "../common/mock_fetch.rs"]
mod mock_fetch;

mod deepseek;
