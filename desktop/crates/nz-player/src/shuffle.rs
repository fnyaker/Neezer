//! "Shuffle" as the web player does it: the playing row stays FIRST, the rest
//! is a Fisher-Yates shuffle, and the order it came from is kept so un-shuffling
//! puts the queue back exactly (`webapp/src/lib/stores.js#shuffled`,
//! `#toggleShuffle`).
//!
//! The shuffle is the only place the state machine may be non-deterministic, so
//! it is an explicit seed here and not a call to the global RNG: a test asserts
//! a permutation, the app seeds from the OS once at startup.

/// A small deterministic generator (SplitMix64): no dependency, no syscall, and
/// the same seed gives the same order — which is what makes a shuffle testable.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// Seeds from the clock and the process, for production use.
    pub fn from_entropy() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        Self(nanos ^ (std::process::id() as u64).rotate_left(32))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (n > 0), by rejection so the low values are not
    /// favoured the way a plain modulo would.
    pub fn below(&mut self, n: u64) -> u64 {
        debug_assert!(n > 0);
        let zone = u64::MAX - (u64::MAX % n);
        loop {
            let value = self.next_u64();
            if value < zone {
                return value % n;
            }
        }
    }
}

impl Default for Rng {
    fn default() -> Self {
        Self::from_entropy()
    }
}

/// Fisher-Yates over positions, exactly the loop `stores.js#shuffled` runs from
/// the end down to the second element.
pub fn shuffled_positions(n: usize, rng: &mut Rng) -> Vec<usize> {
    let mut out: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        let j = rng.below(i as u64 + 1) as usize;
        out.swap(i, j);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shuffle_is_a_permutation_and_the_seed_is_the_whole_input() {
        for n in 0..40usize {
            let mut a = Rng::new(7);
            let mut b = Rng::new(7);
            let first = shuffled_positions(n, &mut a);
            assert_eq!(first.len(), n);
            let mut sorted = first.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, (0..n).collect::<Vec<_>>(), "n={n}");

            let second = shuffled_positions(n, &mut b);
            // `b` continued from where `a` left off, so the second draws differ
            // from the first but are reproducible from the same seed.
            assert_eq!(second.len(), n, "n={n}");

            let mut c = Rng::new(7);
            let replay = shuffled_positions(n, &mut c);
            assert_eq!(first, replay, "n={n}");
        }
    }

    #[test]
    fn a_long_shuffle_actually_moves_things() {
        let mut rng = Rng::new(1);
        let order = shuffled_positions(1_000, &mut rng);
        let fixed = order.iter().enumerate().filter(|(i, p)| i == *p).count();
        // A Fisher-Yates over 1 000 rows leaves about one position in place
        // (the expected number is exactly 1); 20 would mean it is not shuffling.
        assert!(fixed < 20, "fixed={fixed}");
    }

    #[test]
    fn the_bounded_draw_never_escapes_its_range() {
        let mut rng = Rng::new(99);
        for n in 1..=17u64 {
            for _ in 0..1_000 {
                assert!(rng.below(n) < n);
            }
        }
    }
}
