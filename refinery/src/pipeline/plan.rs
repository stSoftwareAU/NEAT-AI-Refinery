//! Turning a configuration into the validated stages a run applies.
//!
//! Planning happens once, before any corpus file is opened: every stage
//! parameter is parsed by the transform that owns it, so a misspelt scheme or
//! an impossible noise policy is refused with the transform's own explanation
//! rather than part way through a run with a corpus already staged.

use std::path::PathBuf;

use super::{FuzzStage, PipelineConfig, PipelineError, PipelineStage, SampleStage, StageError};
use crate::corpus::RecordShape;
use crate::fuzz::{FuzzBounds, FuzzPolicy};
use crate::manifest::CallerMetadata;
use crate::quantise::QuantiseScheme;
use crate::sample::SampleRate;

/// One pipeline run, fully specified.
#[derive(Debug, Clone)]
pub struct PipelineRequest {
    /// The source corpus directory, scanned for `.bin` files. Read-only.
    pub source: PathBuf,
    /// The derived corpus directory to publish, replaced whole.
    pub output: PathBuf,
    /// The record layout of the source corpus. A stage that changes the layout
    /// hands the changed one to the next stage.
    pub shape: RecordShape,
    /// The stages to apply, in order.
    pub config: PipelineConfig,
    /// Opaque caller metadata to record in the published manifest.
    pub metadata: CallerMetadata,
}

/// One validated stage, with the seed it will actually run under.
#[derive(Debug, Clone)]
pub struct PlannedStage {
    /// Its position in the pipeline, counting from one.
    pub position: usize,
    /// The transform and its validated parameters.
    pub kind: StageKind,
    /// The seed the stage runs under, for a transform that draws randomness.
    pub seed: Option<u64>,
}

impl PlannedStage {
    /// The transform name, as the manifest records it.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.kind.name()
    }

    /// The scratch directory name this stage publishes into.
    ///
    /// The position leads, so the intermediate corpora of a run sort into the
    /// order they were produced in.
    #[must_use]
    pub fn directory_name(&self) -> String {
        format!("stage-{:02}-{}", self.position, self.name())
    }
}

/// A transform and the parameters it was validated with.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum StageKind {
    /// Materialised sampling at a validated rate.
    Sample(SampleRate),
    /// Seeded noise under a validated policy.
    Fuzz(FuzzPolicy),
    /// Re-encoding under a known scheme.
    Quantise(QuantiseScheme),
}

impl StageKind {
    /// The transform name, as the manifest records it.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Sample(_) => "sample",
            Self::Fuzz(_) => "fuzz",
            Self::Quantise(_) => "quantise",
        }
    }
}

impl PipelineConfig {
    /// Validates every stage and resolves the seed each one runs under.
    ///
    /// A stage that pins its own seed keeps it; every other stage draws one
    /// derived from `seed` and its position, so no two stages share a
    /// sequence and moving a stage changes what it draws.
    ///
    /// # Errors
    ///
    /// Returns [`PipelineError::Stage`] for a stage whose parameters the
    /// owning transform refuses.
    pub fn plan(&self, seed: u64) -> Result<Vec<PlannedStage>, PipelineError> {
        self.stages
            .iter()
            .enumerate()
            .map(|(index, stage)| {
                let position = index + 1;
                let kind = plan_stage(stage)
                    .map_err(|error| PipelineError::stage(position, stage.name(), error))?;
                let seed = match kind {
                    StageKind::Quantise(_) => None,
                    _ => Some(stage.seed().unwrap_or_else(|| stage_seed(seed, position))),
                };
                Ok(PlannedStage {
                    position,
                    kind,
                    seed,
                })
            })
            .collect()
    }
}

/// Validates one stage's parameters through the transform that owns them.
fn plan_stage(stage: &PipelineStage) -> Result<StageKind, StageError> {
    match stage {
        PipelineStage::Sample(SampleStage { rate, .. }) => {
            Ok(StageKind::Sample(SampleRate::new(*rate)?))
        }
        PipelineStage::Fuzz(FuzzStage {
            distribution,
            scale,
            mode,
            targets,
            clamp_min,
            clamp_max,
            ..
        }) => Ok(StageKind::Fuzz(FuzzPolicy::new(
            distribution.parse()?,
            *scale,
            mode.parse()?,
            targets.parse()?,
            FuzzBounds::new(*clamp_min, *clamp_max)?,
        )?)),
        PipelineStage::Quantise(stage) => Ok(StageKind::Quantise(stage.scheme.parse()?)),
    }
}

/// The seed stage `position` draws under, given the pipeline seed.
///
/// This is the SplitMix64 finaliser over the pipeline seed mixed with the
/// position: a fixed, documented function, so the same pipeline seed always
/// yields the same stage seeds on every machine, and two stages of one run
/// never share a draw sequence.
fn stage_seed(pipeline_seed: u64, position: usize) -> u64 {
    const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

    let mut z = pipeline_seed.wrapping_add((position as u64).wrapping_mul(GOLDEN_GAMMA));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::QuantiseStage;

    fn sample(rate: f64, seed: Option<u64>) -> PipelineStage {
        PipelineStage::Sample(SampleStage { rate, seed })
    }

    fn fuzz() -> PipelineStage {
        PipelineStage::Fuzz(FuzzStage {
            distribution: "gaussian".to_string(),
            scale: 0.01,
            mode: "relative".to_string(),
            targets: "all".to_string(),
            clamp_min: None,
            clamp_max: None,
            seed: None,
        })
    }

    fn quantise() -> PipelineStage {
        PipelineStage::Quantise(QuantiseStage {
            scheme: "bfloat16".to_string(),
        })
    }

    #[test]
    fn numbers_the_stages_from_one_in_configuration_order() {
        let planned = PipelineConfig::new(vec![sample(0.5, None), fuzz(), quantise()])
            .plan(11)
            .expect("every stage is valid");

        assert_eq!(
            planned
                .iter()
                .map(|stage| (stage.position, stage.name()))
                .collect::<Vec<_>>(),
            vec![(1, "sample"), (2, "fuzz"), (3, "quantise")]
        );
        assert_eq!(planned[0].directory_name(), "stage-01-sample");
        assert_eq!(planned[2].directory_name(), "stage-03-quantise");
    }

    #[test]
    fn gives_each_seeded_stage_its_own_derived_seed() {
        let planned = PipelineConfig::new(vec![sample(0.5, None), fuzz(), quantise()])
            .plan(11)
            .expect("every stage is valid");

        assert_ne!(planned[0].seed, planned[1].seed);
        assert_ne!(planned[0].seed, Some(11), "the pipeline seed is not reused");
        assert_eq!(planned[2].seed, None, "quantise draws nothing");
    }

    #[test]
    fn derives_the_same_stage_seeds_from_the_same_pipeline_seed() {
        let stages = || vec![sample(0.5, None), fuzz()];
        let first = PipelineConfig::new(stages()).plan(42).expect("valid");
        let second = PipelineConfig::new(stages()).plan(42).expect("valid");
        let other = PipelineConfig::new(stages()).plan(43).expect("valid");

        assert_eq!(first[0].seed, second[0].seed);
        assert_eq!(first[1].seed, second[1].seed);
        assert_ne!(first[0].seed, other[0].seed);
    }

    #[test]
    fn moving_a_stage_changes_the_seed_it_draws_under() {
        let leading = PipelineConfig::new(vec![fuzz(), sample(0.5, None)])
            .plan(42)
            .expect("valid");
        let trailing = PipelineConfig::new(vec![sample(0.5, None), fuzz()])
            .plan(42)
            .expect("valid");

        assert_ne!(
            leading[0].seed, trailing[1].seed,
            "the position is part of the derivation, so order is never silently equivalent"
        );
    }

    #[test]
    fn keeps_a_seed_a_stage_pins_for_itself() {
        let planned = PipelineConfig::new(vec![sample(0.5, Some(1234))])
            .plan(42)
            .expect("valid");

        assert_eq!(planned[0].seed, Some(1234));
    }

    #[test]
    fn names_the_position_of_a_stage_it_refuses() {
        let error = PipelineConfig::new(vec![
            quantise(),
            PipelineStage::Sample(SampleStage {
                rate: 0.0,
                seed: None,
            }),
        ])
        .plan(1)
        .expect_err("a rate of zero keeps nothing");

        assert!(
            matches!(
                error,
                PipelineError::Stage {
                    position: 2,
                    ref name,
                    ..
                } if name == "sample"
            ),
            "{error:?}"
        );
    }
}
