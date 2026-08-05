//! Cache-hit probing infrastructure (RFC-0015) — layer ① of the three-layer
//! split: probe itself. Collects fingerprints, judges verdicts, stores
//! hashes, and exposes query APIs. Business logic (alerts / CLI / reports)
//! lives outside this module.
//!
//! - [`fingerprint`]: keyed block-hash chains + request-body denoising
//! - [`record`]: `TraceRecord` data model (plaintext-free)
//! - [`verdict`]: rules engine (8 hard invariants, strict/shared dual mode)
//! - [`store`]: `TraceSink` + bounded `RingTraceStore` + query API + JSONL
//! - [`layer`]: `TraceLayer` decorator + `CacheAuditor` trait

pub mod fingerprint;
pub mod hash;
pub mod layer;
pub mod record;
pub mod store;
pub mod verdict;

pub use fingerprint::{BlockChainFingerprint, Chain, Fingerprint, denoise};
pub use layer::{CacheAuditor, RuleAuditor, TraceLayer};
pub use record::{RequestCacheHints, TraceRecord, UsageSnapshot};
pub use store::{
    BreakKind, PrefixBreak, RingTraceStore, SessionChainView, TraceFilter, TraceSink, TraceStats,
};
pub use verdict::{
    JudgmentInput, LcpInput, ProviderAuditSpec, ProviderFamily, SessionStats, Verdict,
    VerdictConfidence, VerdictKind, judge, matrix, quantize_down,
};
