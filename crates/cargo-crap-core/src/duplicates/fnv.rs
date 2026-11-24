//! Stable FNV-1a hashing for structural fingerprints and legacy verdict keys.

use std::hash::Hasher;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a as a [`Hasher`], so the derived `Hash` impls do the walking.
/// Shared with the triage cache, whose keys must be just as stable.
#[derive(Debug)]
pub struct Fnv1a {
    state: u64,
}

impl Default for Fnv1a {
    /// Start a new hash at the FNV offset basis.
    fn default() -> Self {
        Self { state: FNV_OFFSET }
    }
}

impl Hasher for Fnv1a {
    fn write(
        &mut self,
        bytes: &[u8],
    ) {
        for b in bytes {
            self.state ^= u64::from(*b);
            self.state = self.state.wrapping_mul(FNV_PRIME);
        }
    }

    fn finish(&self) -> u64 {
        self.state
    }
}
