use std::hash::{BuildHasher, RandomState};
use std::sync::atomic::{AtomicU64, Ordering};

/// Generate a non-cryptographic, 16-character alphanumeric ID, like `generateId`.
#[must_use]
pub fn generate_id() -> String {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let random = RandomState::new();
    (0..16)
        .map(|index| {
            let value = random.hash_one((counter, index));
            char::from(ALPHABET[(value % ALPHABET.len() as u64) as usize])
        })
        .collect()
}
