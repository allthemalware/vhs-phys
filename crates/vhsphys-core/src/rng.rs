//! Deterministic random numbers. Every stochastic process in the simulator is
//! seeded from a (seed, stream, index) triple so that results do not depend on
//! thread scheduling or on the order fields are processed.

use rand::rngs::SmallRng;
use rand::{Rng as _, SeedableRng};
use rand_distr::StandardNormal;

/// SplitMix64 finaliser, used to hash seed components together.
pub fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub fn hash3(a: u64, b: u64, c: u64) -> u64 {
    mix(mix(mix(a) ^ b.wrapping_mul(0xD6E8_FEB8_6659_FD93)) ^ c.wrapping_mul(0xA076_1D64_78BD_642F))
}

pub struct Rng(SmallRng);

impl Rng {
    pub fn new(seed: u64, stream: u64, index: i64) -> Self {
        Self(SmallRng::seed_from_u64(hash3(seed, stream, index as u64)))
    }

    #[inline(always)]
    pub fn gauss(&mut self) -> f32 {
        self.0.sample(StandardNormal)
    }

    #[inline(always)]
    pub fn uniform(&mut self) -> f64 {
        self.0.random::<f64>()
    }

    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.uniform()
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.0.random_range(0..n.max(1))
    }

    /// Poisson-distributed count with mean `lambda` (Knuth for small means,
    /// normal approximation above 30).
    pub fn poisson(&mut self, lambda: f64) -> u64 {
        if lambda <= 0.0 {
            return 0;
        }
        if lambda > 30.0 {
            let v = lambda + lambda.sqrt() * self.gauss() as f64;
            return v.round().max(0.0) as u64;
        }
        let l = (-lambda).exp();
        let (mut k, mut p) = (0u64, 1.0f64);
        loop {
            p *= self.uniform();
            if p <= l {
                return k;
            }
            k += 1;
        }
    }

    /// Power-law sample on [lo, hi] with pdf proportional to x^-alpha.
    pub fn power_law(&mut self, alpha: f64, lo: f64, hi: f64) -> f64 {
        let u = self.uniform();
        if (alpha - 1.0).abs() < 1e-9 {
            return lo * (hi / lo).powf(u);
        }
        let e = 1.0 - alpha;
        (lo.powf(e) + u * (hi.powf(e) - lo.powf(e))).powf(1.0 / e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_stream_independent() {
        let a: Vec<f32> = { let mut r = Rng::new(1, 2, 3); (0..4).map(|_| r.gauss()).collect() };
        let b: Vec<f32> = { let mut r = Rng::new(1, 2, 3); (0..4).map(|_| r.gauss()).collect() };
        let c: Vec<f32> = { let mut r = Rng::new(1, 2, 4); (0..4).map(|_| r.gauss()).collect() };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn poisson_mean_and_power_law_bounds() {
        let mut r = Rng::new(7, 0, 0);
        let n = 20000;
        let m: f64 = (0..n).map(|_| r.poisson(3.0) as f64).sum::<f64>() / n as f64;
        assert!((m - 3.0).abs() < 0.1, "{m}");
        for _ in 0..1000 {
            let v = r.power_law(2.0, 1.0, 50.0);
            assert!((1.0..=50.0).contains(&v));
        }
    }
}
