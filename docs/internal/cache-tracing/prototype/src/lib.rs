//! Round-4 缓存审计核心算法原型:块哈希链指纹 + TraceStore + 判定引擎。
//! 独立工程,零外部依赖。见 docs/internal/cache-tracing/rounds/round-3-*.md。

pub mod fingerprint;
pub mod hash;
pub mod store;
pub mod synth;
pub mod verdict;

pub use fingerprint::{BlockChainFingerprint, Chain};
pub use store::{LcpResult, MatchInfo, StoredRecord, TraceStore};
pub use verdict::{judge, Confidence, Family, JudgeInput, Kind, LcpInput, ProviderSpec, Verdict};
