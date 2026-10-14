//! Utility functions ported from the Vercel AI SDK TypeScript `util` package.
//!
//! This module is a facade — the implementations live in dedicated modules:
//!
//! - `json_repair` — `fix_json` + `PartialJson::parse` (from `fix-json.ts`
//!   and `parse-partial-json.ts`)
//! - `math` — `cosine_similarity` (from `cosine-similarity.ts`)
//!
//! The re-exports below keep `aimux_core::util::*` paths working for existing
//! consumers.

pub use crate::json_repair::{PartialJson, fix_json};
pub use crate::math::{UtilError, cosine_similarity};
