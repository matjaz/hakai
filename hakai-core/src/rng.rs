//! SplitMix64 — a tiny deterministic generator.
//!
//! Ported from `SeededRNG.swift`. Deterministic randomness is a requirement here, not a
//! convenience: procedural decals must be identical for the same seed, otherwise they
//! can't be regression-tested against a hash of their pixels.

#[derive(Clone, Debug)]
pub struct SeededRng {
    state: u64,
}

impl SeededRng {
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0x9E3779B97F4A7C15 } else { seed },
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform in `[lo, hi)`.
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + self.unit() * (hi - lo)
    }

    /// Symmetric around zero: `[-m, m)`.
    pub fn jitter(&mut self, m: f64) -> f64 {
        self.range(-m, m)
    }

    /// Uniform integer in `[lo, hi)`. An empty or reversed range returns `lo`, same as the
    /// Swift original — tools don't guard against calling this with `hi <= lo` themselves.
    pub fn int(&mut self, lo: i64, hi: i64) -> i64 {
        if hi <= lo {
            return lo;
        }
        lo + (self.next_u64() % (hi - lo) as u64) as i64
    }

    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn same_seed_gives_same_sequence() {
        let mut a = SeededRng::new(12_345);
        let mut b = SeededRng::new(12_345);
        for _ in 0..200 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut x = SeededRng::new(1);
        let mut y = SeededRng::new(2);
        let xs: Vec<u64> = (0..20).map(|_| x.next_u64()).collect();
        let ys: Vec<u64> = (0..20).map(|_| y.next_u64()).collect();
        assert_ne!(xs, ys);
    }

    #[test]
    fn seed_zero_does_not_degenerate() {
        let mut rng = SeededRng::new(0);
        let values: HashSet<u64> = (0..10).map(|_| rng.next_u64()).collect();
        assert_eq!(values.len(), 10);
    }

    #[test]
    fn all_ranges_stay_in_bounds() {
        let mut rng = SeededRng::new(7);
        for _ in 0..20_000 {
            let u = rng.unit();
            assert!((0.0..1.0).contains(&u));
            let r = rng.range(-3.0, 5.0);
            assert!((-3.0..5.0).contains(&r));
            let j = rng.jitter(2.0);
            assert!((-2.0..2.0).contains(&j));
            let i = rng.int(4, 9);
            assert!((4..9).contains(&i));
        }
    }

    #[test]
    fn int_with_empty_or_reversed_range_returns_lower_bound() {
        let mut rng = SeededRng::new(3);
        assert_eq!(rng.int(5, 5), 5);
        assert_eq!(rng.int(5, 2), 5);
    }
}
