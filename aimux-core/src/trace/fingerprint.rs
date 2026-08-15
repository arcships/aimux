//! Block-chain fingerprint + request-body denoising (RFC-0015 §3).
//!
//! Chain construction: `h_i = H128(key_i, block_i)` with
//! `key_i = mix(scope_salt ^ low64(h_{i-1}))` — `h_i` depends on
//! (scope_salt, h_{i-1}, block_i). The last block may be partial; blocks are
//! hashed as-is (no padding). Plaintext bodies never persist — only the chain
//! and counters do.
//!
//! Denoising: the audit baseline is the *denoised* request body — random
//! request ids / timestamps / nonces are stripped before fingerprinting so
//! they cannot break prefix continuation across calls (RFC-0015 §3).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use super::hash;

/// Default block size (4 KiB, matching the prototype).
pub const DEFAULT_BLOCK_SIZE: usize = 4096;

/// JSON keys treated as noise and stripped before fingerprinting
/// (request_id / timestamp / nonce families, RFC-0015 §3).
pub const NOISE_KEYS: &[&str] = &["request_id", "requestId", "timestamp", "nonce"];

/// A block-hash chain for one body (the byte-side core of a fingerprint).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chain {
    /// 128-bit hash of the whole (denoised) body — dedup/retry detection.
    pub body_hash: u128,
    pub len_bytes: u64,
    pub block_size: usize,
    /// `h_i = H(key_i, block_i)`, `i ∈ [0, block_count)`.
    pub block_hashes: Vec<u128>,
}

impl Chain {
    #[must_use]
    pub fn block_count(&self) -> u64 {
        self.block_hashes.len() as u64
    }
}

/// Computes keyed block-hash chains for one scope.
pub struct BlockChainFingerprint {
    pub block_size: usize,
    /// Per-scope salt (process-random master key XOR scope derivation).
    pub scope_salt: u64,
}

impl BlockChainFingerprint {
    #[must_use]
    pub fn new(block_size: usize, scope_salt: u64) -> Self {
        debug_assert!(block_size >= 16, "block size too small");
        Self {
            block_size,
            scope_salt,
        }
    }

    /// Compute the chain for a byte slice.
    #[must_use]
    pub fn compute(&self, data: &[u8]) -> Chain {
        let n = data.len();
        let mut prev: u128 = 0;
        let mut hashes = Vec::with_capacity(n / self.block_size + 1);
        for chunk in data.chunks(self.block_size) {
            let key = hash::mix(self.scope_salt ^ hash::low64(prev));
            let h = hash::hash128(key, chunk);
            hashes.push(h);
            prev = h;
        }
        let body_hash = hash::hash128(hash::mix(self.scope_salt ^ 0x517c_c1b7_2722_0a95), data);
        Chain {
            body_hash,
            len_bytes: n as u64,
            block_size: self.block_size,
            block_hashes: hashes,
        }
    }
}

/// Remove noise keys from a JSON value (in place on a clone).
///
/// Conservative by design (RFC-0015 §3): random request-identity fields are
/// stripped so they cannot break prefix continuation across calls, while
/// business fields — message content, tool arguments, options — are always
/// preserved even when they share a name with a noise key.
///
/// - Top level: `request_id` / `requestId` / `timestamp` / `nonce` (wire
///   transport metadata).
/// - Nested levels: only `request_id` / `requestId` (universal request ids;
///   `timestamp`/`nonce` may be legitimate business payload inside tool
///   arguments, so they are kept there).
#[must_use]
pub fn denoise(value: &Value) -> Value {
    denoise_at(value, 0)
}

fn denoise_at(value: &Value, depth: usize) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, v) in map {
                if depth == 0 {
                    // Top level: wire transport metadata is stripped.
                    if NOISE_KEYS.contains(&k.as_str()) {
                        continue;
                    }
                } else if k == "request_id" || k == "requestId" {
                    // Nested: only universal request ids; timestamp/nonce
                    // may be legitimate business payload (tool arguments).
                    continue;
                }
                out.insert(k.clone(), denoise_at(v, depth + 1));
            }
            Value::Object(out)
        }
        Value::Array(items) => {
            Value::Array(items.iter().map(|i| denoise_at(i, depth + 1)).collect())
        }
        other => other.clone(),
    }
}

/// Serialized-wire fingerprint stored in a `TraceRecord` (hex strings —
/// `u128` is not JSON-serializable).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Fingerprint {
    /// 128-bit hash of the whole denoised body (hex).
    pub body_hash: String,
    pub len_bytes: u64,
    pub block_size: u64,
    /// Block-hash chain (hex), one entry per block.
    pub block_hashes: Vec<String>,
    /// Token estimate — byte proxy `len_bytes / 4` when no tokenizer is
    /// attached (RFC-0015 §4.1: W capped at one block in this mode).
    pub token_estimate: u64,
}

impl From<&Chain> for Fingerprint {
    fn from(chain: &Chain) -> Self {
        Fingerprint {
            body_hash: hash::hex128(chain.body_hash),
            len_bytes: chain.len_bytes,
            block_size: chain.block_size as u64,
            block_hashes: chain
                .block_hashes
                .iter()
                .map(|h| hash::hex128(*h))
                .collect(),
            token_estimate: chain.len_bytes / 4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_deterministic_and_sensitive() {
        let fp = BlockChainFingerprint::new(512, 7);
        let a = b"hello world".repeat(64);
        let b = b"hello world".repeat(64);
        let c = b"hello worle".repeat(64); // one byte differs
        assert_eq!(fp.compute(&a).body_hash, fp.compute(&b).body_hash);
        assert_ne!(fp.compute(&a).body_hash, fp.compute(&c).body_hash);
    }

    #[test]
    fn block_count_matches_chunks() {
        let fp = BlockChainFingerprint::new(512, 1);
        let data = vec![b'x'; 1200];
        let chain = fp.compute(&data);
        assert_eq!(chain.block_count(), 3);
        assert_eq!(chain.len_bytes, 1200);
    }

    #[test]
    fn denoise_strips_top_level_noise_and_nested_request_ids() {
        let v = serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "request_id": "rand-1",
            "timestamp": 123,
            "nonce": "n1",
            "nested": {"nonce": "abc", "keep": 1},
            "array": [{"requestId": "x"}, {"keep": 2}],
        });
        let d = denoise(&v);
        assert!(d.get("request_id").is_none());
        assert!(d.get("timestamp").is_none());
        assert!(d.get("nonce").is_none());
        assert_eq!(d["nested"]["keep"], 1);
        // Nested timestamp/nonce are BUSINESS payload — preserved.
        assert_eq!(d["nested"]["nonce"], "abc");
        // Nested universal request ids are still stripped.
        assert!(d["array"][0].get("requestId").is_none());
        assert_eq!(d["array"][1]["keep"], 2);
        assert_eq!(d["model"], "gpt-4o");
        assert_eq!(d["messages"][0]["content"], "hi");
    }

    #[test]
    fn denoise_preserves_tool_arguments_with_timestamp_and_nonce() {
        // Business fields that merely share noise-key names must survive —
        // deleting them would change the token prefix the provider sees.
        let v = serde_json::json!({
            "messages": [{
                "role": "user",
                "content": "hi",
                "tool_arguments": {"timestamp": 123456, "nonce": "sig-1", "data": "x"}
            }]
        });
        let d = denoise(&v);
        assert_eq!(d["messages"][0]["tool_arguments"]["timestamp"], 123456);
        assert_eq!(d["messages"][0]["tool_arguments"]["nonce"], "sig-1");
    }

    #[test]
    fn denoise_preserves_body_fingerprint_stability() {
        // Two calls whose only difference is a noise key must produce the
        // same chain (prefix continuation is not broken by random ids).
        let fp = BlockChainFingerprint::new(512, 42);
        let v1 = serde_json::json!({"messages":[{"role":"user","content":"abc"}],"request_id":"r1","timestamp":1});
        let v2 = serde_json::json!({"messages":[{"role":"user","content":"abc"}],"request_id":"r2","timestamp":2});
        let b1 = serde_json::to_vec(&denoise(&v1)).unwrap();
        let b2 = serde_json::to_vec(&denoise(&v2)).unwrap();
        assert_eq!(b1, b2);
        assert_eq!(fp.compute(&b1).body_hash, fp.compute(&b2).body_hash);
    }

    #[test]
    fn fingerprint_serializes_as_hex_strings() {
        let fp = BlockChainFingerprint::new(512, 3);
        let chain = fp.compute(b"data");
        let f = Fingerprint::from(&chain);
        assert_eq!(f.body_hash.len(), 32); // 128-bit hex
        assert!(f.block_hashes.iter().all(|h| h.len() == 32));
        let json = serde_json::to_string(&f).unwrap();
        assert!(json.contains("\"body_hash\":\""));
    }
}
