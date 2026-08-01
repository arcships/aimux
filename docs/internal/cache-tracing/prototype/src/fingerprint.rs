//! BlockChainFingerprint:块哈希链(D5 算法原型)。
//!
//! 链式构造:`h_i = H128(key_i, block_i)`,其中 `key_i = mix(scope_salt ^ low64(h_{i-1}))`,
//! 即 h_i 依赖 (scope_salt, h_{i-1}, block_i) 三者——满足
//! `h_i = hash(key, h_{i-1}, block_i)` 的组合语义(具体混合方式为实现细节)。
//!
//! 约定:最后一个块可为部分块(短 body/尾块);块内容按实际字节哈希,不做填充。
//! 明文 body 永不落盘,只存链 + 计数。

use crate::hash;

pub const DEFAULT_BLOCK_SIZE: usize = 4096;

/// 一次链计算的结果(即 Fingerprint 的字节侧核心)。
#[derive(Debug, Clone)]
pub struct Chain {
    /// 全 body 的 128-bit 哈希(dedup/retry 检测用,对应 Fingerprint.body_hash)。
    pub body_hash: u128,
    pub len_bytes: u64,
    pub block_size: usize,
    /// h_i = H(key_i, block_i),i ∈ [0, block_count)。
    pub block_hashes: Vec<u128>,
    pub truncated: bool,
}

impl Chain {
    pub fn block_count(&self) -> u64 {
        self.block_hashes.len() as u64
    }
}

pub struct BlockChainFingerprint {
    pub block_size: usize,
    /// 每 scope 一把盐(进程随机 master key ⊕ scope 盐的简化:直接取 scope 派生值)。
    pub scope_salt: u64,
}

impl BlockChainFingerprint {
    pub fn new(block_size: usize, scope_salt: u64) -> Self {
        debug_assert!(block_size >= 16, "block size too small");
        Self {
            block_size,
            scope_salt,
        }
    }

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
            truncated: false,
        }
    }
}
