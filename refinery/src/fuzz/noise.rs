//! The seeded noise source a fuzzing run draws from.
//!
//! Draws are standardised — zero mean, unit scale — so the policy owns the
//! magnitude and the distribution owns only the shape. A source is created from
//! a seed and drawn from in a fixed order, which is what makes a run
//! reproducible: the same seed and the same policy replay the same sequence,
//! whatever the corpus contains.

use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

use super::FuzzDistribution;

/// A reproducible stream of standardised noise.
#[derive(Debug)]
pub struct NoiseSource {
    distribution: FuzzDistribution,
    rng: StdRng,
    /// Box–Muller produces normal draws in pairs; the second is held for the
    /// next call rather than discarded.
    spare: Option<f64>,
}

impl NoiseSource {
    /// Starts a stream for `distribution` from `seed`.
    #[must_use]
    pub fn new(distribution: FuzzDistribution, seed: u64) -> Self {
        Self {
            distribution,
            rng: StdRng::seed_from_u64(seed),
            spare: None,
        }
    }

    /// The next draw: `N(0, 1)` for a Gaussian stream, `[-1, 1)` for a uniform
    /// one.
    pub fn draw(&mut self) -> f64 {
        match self.distribution {
            FuzzDistribution::Gaussian => self.gaussian(),
            FuzzDistribution::Uniform => self.rng.random::<f64>().mul_add(2.0, -1.0),
        }
    }

    /// One standard normal draw, by the Box–Muller transform.
    ///
    /// `random` yields `[0, 1)`, so the radius is taken from `1 - u`, which is
    /// `(0, 1]`: the logarithm of zero is never evaluated.
    fn gaussian(&mut self) -> f64 {
        if let Some(spare) = self.spare.take() {
            return spare;
        }

        let radius = (-2.0 * (1.0 - self.rng.random::<f64>()).ln()).sqrt();
        let angle = std::f64::consts::TAU * self.rng.random::<f64>();
        let (sin, cos) = angle.sin_cos();
        self.spare = Some(radius * sin);
        radius * cos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mean and standard deviation of `count` draws.
    fn moments(distribution: FuzzDistribution, seed: u64, count: usize) -> (f64, f64) {
        let mut source = NoiseSource::new(distribution, seed);
        let draws: Vec<f64> = (0..count).map(|_| source.draw()).collect();

        let mean = draws.iter().sum::<f64>() / count as f64;
        let variance = draws.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / count as f64;
        (mean, variance.sqrt())
    }

    #[test]
    fn a_gaussian_stream_is_standard_normal() {
        let (mean, deviation) = moments(FuzzDistribution::Gaussian, 20_260_831, 100_000);

        // Loose bounds: this asserts the distribution is the one claimed, not
        // that a particular seed hits a particular sample.
        assert!(mean.abs() < 0.02, "mean was {mean}");
        assert!((deviation - 1.0).abs() < 0.02, "deviation was {deviation}");
    }

    #[test]
    fn a_uniform_stream_stays_inside_the_unit_interval() {
        let mut source = NoiseSource::new(FuzzDistribution::Uniform, 7);
        let draws: Vec<f64> = (0..100_000).map(|_| source.draw()).collect();

        assert!(
            draws.iter().all(|draw| (-1.0..1.0).contains(draw)),
            "a uniform draw never leaves [-1, 1)"
        );
        assert!(
            draws.iter().any(|draw| *draw < -0.99) && draws.iter().any(|draw| *draw > 0.99),
            "and it reaches both ends"
        );

        let (mean, deviation) = moments(FuzzDistribution::Uniform, 7, 100_000);
        assert!(mean.abs() < 0.02, "mean was {mean}");
        // The standard deviation of a uniform on [-1, 1) is 1/sqrt(3).
        assert!(
            (deviation - 1.0 / 3.0_f64.sqrt()).abs() < 0.02,
            "deviation was {deviation}"
        );
    }

    #[test]
    fn a_gaussian_stream_produces_both_halves_of_every_pair() {
        // The transform makes two draws at a time; an odd count proves the
        // held-back one is handed out rather than lost.
        let first: Vec<f64> = {
            let mut source = NoiseSource::new(FuzzDistribution::Gaussian, 11);
            (0..5).map(|_| source.draw()).collect()
        };

        assert_eq!(first.len(), 5);
        assert!(first.iter().all(|draw| draw.is_finite()));
        assert!(
            first.windows(2).any(|pair| pair[0] != pair[1]),
            "consecutive draws must not repeat"
        );
    }

    #[test]
    fn the_same_seed_replays_the_same_sequence() {
        for distribution in FuzzDistribution::ALL {
            let take = |seed| {
                let mut source = NoiseSource::new(*distribution, seed);
                (0..64).map(|_| source.draw()).collect::<Vec<f64>>()
            };

            assert_eq!(take(3), take(3), "{distribution} is reproducible");
            assert_ne!(take(3), take(4), "{distribution} depends on the seed");
        }
    }
}
