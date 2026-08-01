//! Keyed 64-bit hash (splitmix64 风格折叠),用两个不同派生 key 各算一遍组成
//! 128-bit 指纹(任务允许的"两个 64-bit 混合",零外部依赖,替代 xxh3-128)。
//!
//! 注意:非密码学哈希。128-bit 宽度下对随机碰撞足够(≈2^-128);
//! 抗恶意构造不在原型范围(生产路径应换 xxh3-128 或密钥化更强哈希)。

#[inline(always)]
pub fn mix(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    x
}

/// 带 key 的 64-bit 哈希:8 字节字折叠 + 旋转 + 终末化(含长度前缀)。
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

/// 128-bit 键控哈希 = 两个不同派生 key 的 64-bit 混合。
pub fn hash128(key: u64, data: &[u8]) -> u128 {
    let k1 = mix(key ^ 0x243f_6a88_85a3_08d3);
    let k2 = mix(k1 ^ 0x1319_8a2e_0370_7344);
    ((hash64(k1, data) as u128) << 64) | (hash64(k2, data) as u128)
}

/// 取 128-bit 哈希低 64 位(反向索引键;碰撞靠全 128-bit 验证拒绝)。
#[inline(always)]
pub fn low64(h: u128) -> u64 {
    (h & 0xffff_ffff_ffff_ffff) as u64
}
