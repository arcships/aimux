//! Keyed hashing for cache-probe fingerprints (RFC-0015 §3).
//!
//! Zero-dependency keyed hashes (splitmix64-style folding). The 128-bit
//! fingerprint is two different derived-key 64-bit hashes — collision odds
//! ~2^-128. Non-cryptographic by design: fingerprints are *keys* in the
//! client-side LCP audit (HMAC-style scope salt), never security tokens.

#[inline(always)]
pub fn mix(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    x
}

/// Keyed 64-bit hash: 8-byte word folding + rotation + finalization (length
/// prefix included).
pub fn hash64(key: u64, data: &[u8]) -> u64 {
    let mut h = key ^ 0x9e37_79b9_7f4a_7c15;
    let mut chunks = data.chunks_exact(8);
    for c in &mut chunks {
        let w = u64::from_le_bytes(c.try_into().unwrap());
        h ^= w.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        h = mix(h.rotate_left(27).wrapping_add(key));
    }
    let rem = chunks.remainder();
    let mut w = data.len() as u64;
    for (i, b) in rem.iter().enumerate() {
        w ^= (*b as u64) << (8 * i);
    }
    h ^= w.wrapping_mul(0xd6e8_feb8_6659_fd93);
    mix(h ^ key.rotate_left(17))
}

/// 128-bit keyed hash = two 64-bit hashes under two derived keys.
pub fn hash128(key: u64, data: &[u8]) -> u128 {
    let k1 = mix(key ^ 0x243f_6a88_85a3_08d3);
    let k2 = mix(k1 ^ 0x1319_8a2e_0370_7344);
    ((hash64(k1, data) as u128) << 64) | (hash64(k2, data) as u128)
}

/// Low 64 bits of a 128-bit hash (reverse-index key; collisions are rejected
/// by full 128-bit chain verification).
#[inline(always)]
pub fn low64(h: u128) -> u64 {
    (h & 0xffff_ffff_ffff_ffff) as u64
}

/// Hex representation of a 128-bit hash (stable across JSON boundaries —
/// `u128` cannot be serialized by serde_json).
pub fn hex128(h: u128) -> String {
    format!("{h:032x}")
}
