//! The validated inputs of a sampling run.

use std::path::PathBuf;

use super::SampleError;
use crate::corpus::RecordShape;

/// A materialised sampling probability.
///
/// The allowed range is the one the Deno sampler enforces — `rate > 0` and
/// `rate <= 1` — so a rate of zero, a negative rate, a rate above one and
/// `NaN` are all rejected before any file is opened.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct SampleRate(f64);

impl SampleRate {
    /// Validates `rate` as a sampling probability.
    ///
    /// # Errors
    ///
    /// Returns [`SampleError::InvalidRate`] for anything outside `0 < rate <= 1`.
    pub fn new(rate: f64) -> Result<Self, SampleError> {
        // Written as the Deno sampler writes it: `NaN` fails both comparisons.
        if rate > 0.0 && rate <= 1.0 {
            Ok(Self(rate))
        } else {
            Err(SampleError::InvalidRate { rate })
        }
    }

    /// The probability itself.
    #[must_use]
    pub const fn value(self) -> f64 {
        self.0
    }

    /// The rate as a whole percentage, rounded half away from zero.
    ///
    /// This matches JavaScript's `Math.round` over the allowed range, so a
    /// given rate names the same file as the Deno sampler.
    #[must_use]
    pub fn percent(self) -> u32 {
        (self.0 * 100.0).round() as u32
    }

    /// The derived corpus file name — `sample-<percent>.bin`.
    #[must_use]
    pub fn file_name(self) -> String {
        format!("sample-{}.bin", self.percent())
    }
}

/// One materialised sampling run, fully specified.
#[derive(Debug, Clone)]
pub struct SampleRequest {
    /// The source corpus directory, scanned for `.bin` files.
    pub source: PathBuf,
    /// The derived corpus directory to publish, replaced whole.
    pub output: PathBuf,
    /// The record layout of both corpora.
    pub shape: RecordShape,
    /// The probability each record is kept.
    pub rate: SampleRate,
    /// A seed for a reproducible run; `None` seeds from the operating system,
    /// which is the production default.
    pub seed: Option<u64>,
}
