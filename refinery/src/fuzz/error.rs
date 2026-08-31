//! Failures of a fuzzing run.
//!
//! Every variant is fatal. A corpus that cannot be perturbed exactly as the
//! stated policy describes is never published approximately: a policy that
//! cannot be applied, or a perturbation that leaves the finite range, aborts
//! the run with nothing published.

use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use super::{FuzzDistribution, FuzzMode, FuzzTargets};
use crate::corpus::CorpusError;
use crate::manifest::ManifestError;
use crate::transform::TransformError;

/// A fuzzing run that could not be completed.
#[derive(Debug)]
#[non_exhaustive]
pub enum FuzzError {
    /// The requested distribution is not one Refinery offers.
    UnknownDistribution {
        /// The distribution name as supplied.
        distribution: String,
    },
    /// The requested application mode is not one Refinery offers.
    UnknownMode {
        /// The mode name as supplied.
        mode: String,
    },
    /// The requested target selection is not one Refinery offers.
    UnknownTargets {
        /// The target selection as supplied.
        targets: String,
    },
    /// The noise scale is not a positive finite number.
    InvalidScale {
        /// The scale as supplied.
        scale: f64,
    },
    /// The bounds cannot hold a value — one is not finite, or they cross.
    InvalidBounds {
        /// The lower bound as supplied.
        min: Option<f32>,
        /// The upper bound as supplied.
        max: Option<f32>,
    },
    /// Perturbing a value produced a result that is not finite.
    ///
    /// A bound does not rescue it: an overflow means the policy does not suit
    /// the corpus, and publishing an infinity in place of a number would hide
    /// that.
    NonFiniteResult {
        /// The record it happened in, counted from zero across the run.
        record: u64,
        /// The value within that record, counted from zero.
        value: usize,
        /// The source value, which was finite.
        original: f32,
        /// What the policy turned it into.
        perturbed: f32,
    },
    /// The source corpus is not encoded the way the run was told to read it.
    ///
    /// Raised from the source's own manifest, so fuzzing a quantised corpus as
    /// if it were `float32` fails loud instead of perturbing reinterpreted
    /// bytes.
    SourceEncodingMismatch {
        /// The manifest that was consulted.
        manifest: PathBuf,
        /// The encoding the run reads.
        expected: String,
        /// The encoding the source manifest declares.
        found: String,
    },
    /// The source corpus declares a record width the caller's shape disagrees
    /// with, so the noise would land on the wrong values.
    SourceWidthMismatch {
        /// The manifest that was consulted.
        manifest: PathBuf,
        /// Bytes per record the caller's `--inputs`/`--outputs` imply.
        expected: usize,
        /// Bytes per record the source manifest declares.
        found: usize,
    },
    /// A step every transform shares failed — discovery, staging, records,
    /// provenance or publication.
    Transform(TransformError),
}

impl From<TransformError> for FuzzError {
    fn from(error: TransformError) -> Self {
        Self::Transform(error)
    }
}

impl From<CorpusError> for FuzzError {
    fn from(error: CorpusError) -> Self {
        Self::Transform(TransformError::Corpus(error))
    }
}

impl From<ManifestError> for FuzzError {
    fn from(error: ManifestError) -> Self {
        Self::Transform(TransformError::Manifest(error))
    }
}

/// The offered names of a choice, for a message that shows the way out.
fn offered(names: impl IntoIterator<Item = &'static str>) -> String {
    names.into_iter().collect::<Vec<_>>().join(", ")
}

impl fmt::Display for FuzzError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownDistribution { distribution } => write!(
                f,
                "unknown noise distribution {distribution:?} — Refinery offers: {}",
                offered(FuzzDistribution::ALL.iter().map(|value| value.name()))
            ),
            Self::UnknownMode { mode } => write!(
                f,
                "unknown fuzz mode {mode:?} — Refinery offers: {}",
                offered(FuzzMode::ALL.iter().map(|value| value.name()))
            ),
            Self::UnknownTargets { targets } => write!(
                f,
                "unknown fuzz targets {targets:?} — Refinery offers: {}",
                offered(FuzzTargets::ALL.iter().map(|value| value.name()))
            ),
            Self::InvalidScale { scale } => write!(
                f,
                "fuzz scale {scale} is not a perturbation — it must be finite and above zero"
            ),
            Self::InvalidBounds { min, max } => write!(
                f,
                "fuzz bounds are unusable — clamp-min {min:?} and clamp-max {max:?} must be finite, with the minimum no higher than the maximum"
            ),
            Self::NonFiniteResult {
                record,
                value,
                original,
                perturbed,
            } => write!(
                f,
                "record {record} value {value}: {original} was perturbed to {perturbed}, which the corpus cannot store — the scale does not suit this corpus"
            ),
            Self::SourceEncodingMismatch {
                manifest,
                expected,
                found,
            } => write!(
                f,
                "source corpus is encoded as {found}, but the run reads {expected} — {} says so",
                manifest.display()
            ),
            Self::SourceWidthMismatch {
                manifest,
                expected,
                found,
            } => write!(
                f,
                "source corpus holds {found} bytes per record, but --inputs/--outputs imply {expected} — {} says so",
                manifest.display()
            ),
            Self::Transform(error) => write!(f, "{error}"),
        }
    }
}

impl Error for FuzzError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Transform(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_alternatives_rather_than_only_the_mistake() {
        let error = FuzzError::UnknownDistribution {
            distribution: "cauchy".to_string(),
        };
        assert!(error.to_string().contains("gaussian"), "{error}");
        assert!(error.to_string().contains("uniform"), "{error}");

        let error = FuzzError::UnknownTargets {
            targets: "everything".to_string(),
        };
        assert!(error.to_string().contains("inputs"), "{error}");
        assert!(error.to_string().contains("all"), "{error}");
    }

    #[test]
    fn locates_a_non_finite_result_in_the_corpus() {
        let error = FuzzError::NonFiniteResult {
            record: 41,
            value: 7,
            original: 1.0,
            perturbed: f32::INFINITY,
        };
        let message = error.to_string();

        assert!(message.contains("record 41"), "{message}");
        assert!(message.contains("value 7"), "{message}");
        assert!(message.contains("inf"), "{message}");
    }
}
