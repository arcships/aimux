//! Utility functions ported from the Vercel AI SDK TypeScript `util` package.
//!
//! This module is a facade — the implementations live in dedicated modules:
//!
//! - [`json_repair`] — `fix_json` + `parse_partial_json` (from `fix-json.ts`
//!   and `parse-partial-json.ts`)
//! - [`math`] — `cosine_similarity` (from `cosine-similarity.ts`)
//!
//! The re-exports below keep `aimux_core::util::*` paths working for existing
//! consumers.

pub use crate::json_repair::{
    ParsePartialJsonResult, ParsePartialJsonState, fix_json, parse_partial_json,
};
pub use crate::math::{UtilError, cosine_similarity};

// ── RFC 3339 时间戳(UTC,毫秒精度;session.rs 同款,统一入口)───────────────

/// Current time as RFC 3339 UTC with millisecond precision
/// (`2026-08-05T04:52:30.123Z`).
pub fn rfc3339_now() -> String {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs() as i64;
    let millis = d.subsec_millis();
    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    let hms = secs.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        hms / 3600,
        (hms % 3600) / 60,
        hms % 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
