//! The validated inputs of a fuzzing run: the perturbation policy, and the
//! request that applies it to a corpus.
//!
//! The policy is deliberately a value rather than a set of loose flags. It is
//! validated once, applied to every value, and recorded verbatim in the
//! manifest, so a derived corpus never leaves the perturbation it carries to be
//! inferred.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use super::FuzzError;
use crate::corpus::RecordShape;
use crate::manifest::CallerMetadata;

/// The shape of the noise a run draws.
///
/// A distribution is named explicitly on the command line and recorded in the
/// manifest. There is no default: the distribution decides the perturbation a
/// corpus carries, so it is always stated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FuzzDistribution {
    /// Zero-mean normal noise, standard deviation one before scaling.
    ///
    /// Unbounded: a draw beyond the scale is rare but possible, which is what
    /// makes it the choice when the tail is the point.
    Gaussian,
    /// Uniform noise on `[-1, 1)` before scaling.
    ///
    /// Bounded by construction: no draw ever exceeds the scale, which is what
    /// makes it the choice when a hard perturbation limit matters.
    Uniform,
}

impl FuzzDistribution {
    /// Every distribution, for a caller listing the choices.
    pub const ALL: &'static [Self] = &[Self::Gaussian, Self::Uniform];

    /// The name the distribution is selected and recorded under.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Gaussian => "gaussian",
            Self::Uniform => "uniform",
        }
    }

    /// The derived corpus file name — `fuzz-<distribution>.bin`.
    #[must_use]
    pub fn file_name(self) -> String {
        format!("fuzz-{}.bin", self.name())
    }
}

impl fmt::Display for FuzzDistribution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for FuzzDistribution {
    type Err = FuzzError;

    /// Parses a distribution name, refusing an unknown one rather than
    /// defaulting.
    fn from_str(distribution: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.name() == distribution)
            .ok_or_else(|| FuzzError::UnknownDistribution {
                distribution: distribution.to_string(),
            })
    }
}

/// How the scaled noise is applied to a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FuzzMode {
    /// `x + scale × n` — the same perturbation whatever the magnitude.
    Absolute,
    /// `x × (1 + scale × n)` — a perturbation proportional to the magnitude,
    /// which leaves an exact zero exactly zero.
    Relative,
}

impl FuzzMode {
    /// Every mode, for a caller listing the choices.
    pub const ALL: &'static [Self] = &[Self::Absolute, Self::Relative];

    /// The name the mode is selected and recorded under.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Absolute => "absolute",
            Self::Relative => "relative",
        }
    }
}

impl fmt::Display for FuzzMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for FuzzMode {
    type Err = FuzzError;

    fn from_str(mode: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.name() == mode)
            .ok_or_else(|| FuzzError::UnknownMode {
                mode: mode.to_string(),
            })
    }
}

/// Which values of a record a run perturbs.
///
/// [`FuzzTargets::Inputs`] is the default, and the reason this type exists: a
/// perturbed expected output silently changes what a corpus is teaching, so
/// reaching one is an explicit request rather than a side effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum FuzzTargets {
    /// The leading `inputs` values of each record — the default.
    #[default]
    Inputs,
    /// The trailing `outputs` values of each record.
    Outputs,
    /// Every value of each record.
    All,
}

impl FuzzTargets {
    /// Every target selection, for a caller listing the choices.
    pub const ALL: &'static [Self] = &[Self::Inputs, Self::Outputs, Self::All];

    /// The name the selection is made and recorded under.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Inputs => "inputs",
            Self::Outputs => "outputs",
            Self::All => "all",
        }
    }

    /// Whether the value at `index` within a record is perturbed.
    ///
    /// A record stores its inputs first and its outputs after them, so the
    /// split is the input count.
    #[must_use]
    pub fn includes(self, index: usize, shape: &RecordShape) -> bool {
        match self {
            Self::Inputs => index < shape.inputs(),
            Self::Outputs => index >= shape.inputs(),
            Self::All => true,
        }
    }
}

impl fmt::Display for FuzzTargets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for FuzzTargets {
    type Err = FuzzError;

    fn from_str(targets: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.name() == targets)
            .ok_or_else(|| FuzzError::UnknownTargets {
                targets: targets.to_string(),
            })
    }
}

/// The range a perturbed value is held inside.
///
/// Either side may be absent, and by default both are: an unbounded run is the
/// honest one when the caller has no domain limit to state. Bounds are stored
/// as `f32` — the width the corpus stores — so a published value is compared
/// against exactly the number that was applied to it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FuzzBounds {
    min: Option<f32>,
    max: Option<f32>,
}

impl FuzzBounds {
    /// Validates a lower and an upper bound.
    ///
    /// # Errors
    ///
    /// Returns [`FuzzError::InvalidBounds`] when either bound is not finite or
    /// the lower one is above the upper one. Equal bounds are allowed: pinning
    /// every perturbed value to one number is degenerate but well defined.
    pub fn new(min: Option<f32>, max: Option<f32>) -> Result<Self, FuzzError> {
        let finite = |bound: Option<f32>| bound.is_none_or(f32::is_finite);
        let ordered = match (min, max) {
            (Some(min), Some(max)) => min <= max,
            _ => true,
        };

        if !finite(min) || !finite(max) || !ordered {
            return Err(FuzzError::InvalidBounds { min, max });
        }
        Ok(Self { min, max })
    }

    /// The lower bound, when there is one.
    #[must_use]
    pub const fn min(self) -> Option<f32> {
        self.min
    }

    /// The upper bound, when there is one.
    #[must_use]
    pub const fn max(self) -> Option<f32> {
        self.max
    }

    /// Holds `value` inside the bounds, reporting whether it had to move.
    ///
    /// A non-finite value is returned untouched: a bound does not rescue a
    /// result that is not a number, and the run refuses it instead.
    #[must_use]
    fn clamp(self, value: f32) -> (f32, bool) {
        if !value.is_finite() {
            return (value, false);
        }
        // Both bounds were proved finite and ordered when this was built, and
        // the value is finite, so `f32::clamp` cannot panic here.
        let held = value.clamp(
            self.min.unwrap_or(f32::NEG_INFINITY),
            self.max.unwrap_or(f32::INFINITY),
        );
        (held, held != value)
    }
}

/// What perturbing one value produced.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Perturbed {
    /// The value to write.
    pub value: f32,
    /// Whether a bound moved it.
    pub clamped: bool,
    /// Whether the source value was not finite and was therefore left exactly
    /// as it was, noise being undefined on it.
    pub preserved: bool,
}

/// The complete perturbation policy of a run.
///
/// Every field is recorded in the manifest, so the policy that produced a
/// derived corpus can be read back off it and applied again.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FuzzPolicy {
    distribution: FuzzDistribution,
    scale: f64,
    mode: FuzzMode,
    targets: FuzzTargets,
    bounds: FuzzBounds,
}

impl FuzzPolicy {
    /// Validates a policy.
    ///
    /// # Errors
    ///
    /// Returns [`FuzzError::InvalidScale`] when `scale` is not a positive
    /// finite number — a scale of zero perturbs nothing and would publish a
    /// copy of the source under a name claiming otherwise.
    pub fn new(
        distribution: FuzzDistribution,
        scale: f64,
        mode: FuzzMode,
        targets: FuzzTargets,
        bounds: FuzzBounds,
    ) -> Result<Self, FuzzError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(FuzzError::InvalidScale { scale });
        }
        Ok(Self {
            distribution,
            scale,
            mode,
            targets,
            bounds,
        })
    }

    /// The distribution the noise is drawn from.
    #[must_use]
    pub const fn distribution(&self) -> FuzzDistribution {
        self.distribution
    }

    /// The scale the drawn noise is multiplied by.
    #[must_use]
    pub const fn scale(&self) -> f64 {
        self.scale
    }

    /// How the scaled noise is applied.
    #[must_use]
    pub const fn mode(&self) -> FuzzMode {
        self.mode
    }

    /// Which values of a record are perturbed.
    #[must_use]
    pub const fn targets(&self) -> FuzzTargets {
        self.targets
    }

    /// The range perturbed values are held inside.
    #[must_use]
    pub const fn bounds(&self) -> FuzzBounds {
        self.bounds
    }

    /// Applies one standardised `noise` draw to `value`.
    ///
    /// The arithmetic is done in `f64` and narrowed once, so a large magnitude
    /// and a small scale do not lose the perturbation to rounding before it is
    /// stored. A source value that is not finite is returned unchanged — noise
    /// is not defined on it — and reported as preserved rather than perturbed.
    #[must_use]
    pub fn perturb(&self, value: f32, noise: f64) -> Perturbed {
        if !value.is_finite() {
            return Perturbed {
                value,
                clamped: false,
                preserved: true,
            };
        }

        let wide = f64::from(value);
        let offset = self.scale * noise;
        let perturbed = match self.mode {
            FuzzMode::Absolute => wide + offset,
            FuzzMode::Relative => wide * (1.0 + offset),
        } as f32;

        let (value, clamped) = self.bounds.clamp(perturbed);
        Perturbed {
            value,
            clamped,
            preserved: false,
        }
    }

    /// The parameters the manifest records, so a run can be repeated exactly.
    ///
    /// Both bounds always appear, `null` when absent: an unbounded policy is a
    /// decision, and a reader must be able to tell it from a field that was
    /// never written.
    #[must_use]
    pub fn parameters(&self) -> BTreeMap<String, serde_json::Value> {
        let bound = |value: Option<f32>| {
            value.map_or(serde_json::Value::Null, |value| {
                serde_json::Value::from(value)
            })
        };

        let mut parameters = BTreeMap::new();
        parameters.insert(
            "distribution".to_string(),
            serde_json::Value::from(self.distribution.name()),
        );
        parameters.insert("scale".to_string(), serde_json::Value::from(self.scale));
        parameters.insert(
            "mode".to_string(),
            serde_json::Value::from(self.mode.name()),
        );
        parameters.insert(
            "targets".to_string(),
            serde_json::Value::from(self.targets.name()),
        );
        parameters.insert("clamp_min".to_string(), bound(self.bounds.min()));
        parameters.insert("clamp_max".to_string(), bound(self.bounds.max()));
        // The two policies that are not flags, so a reader never has to guess
        // what a run did with a value it could not perturb.
        parameters.insert(
            "non_finite_source".to_string(),
            serde_json::Value::from("preserve"),
        );
        parameters.insert(
            "non_finite_result".to_string(),
            serde_json::Value::from("fail"),
        );
        parameters
    }
}

/// One fuzzing run, fully specified.
#[derive(Debug, Clone)]
pub struct FuzzRequest {
    /// The source corpus directory, scanned for `.bin` files.
    pub source: PathBuf,
    /// The derived corpus directory to publish, replaced whole.
    pub output: PathBuf,
    /// The record layout of both corpora — fuzzing does not change it.
    pub shape: RecordShape,
    /// The perturbation applied to every targeted value.
    pub policy: FuzzPolicy,
    /// A seed for a reproducible run; `None` seeds from the operating system.
    pub seed: Option<u64>,
    /// Opaque caller metadata to record in the manifest, uninterpreted.
    pub metadata: CallerMetadata,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(mode: FuzzMode, bounds: FuzzBounds) -> FuzzPolicy {
        FuzzPolicy::new(
            FuzzDistribution::Gaussian,
            0.5,
            mode,
            FuzzTargets::Inputs,
            bounds,
        )
        .expect("a valid policy")
    }

    #[test]
    fn names_the_published_file_after_the_distribution() {
        assert_eq!(FuzzDistribution::Gaussian.file_name(), "fuzz-gaussian.bin");
        assert_eq!(FuzzDistribution::Uniform.file_name(), "fuzz-uniform.bin");
    }

    #[test]
    fn parses_every_offered_name_and_refuses_the_rest() {
        for distribution in FuzzDistribution::ALL {
            assert_eq!(
                distribution
                    .name()
                    .parse::<FuzzDistribution>()
                    .expect("parses"),
                *distribution
            );
        }
        for mode in FuzzMode::ALL {
            assert_eq!(mode.name().parse::<FuzzMode>().expect("parses"), *mode);
        }
        for targets in FuzzTargets::ALL {
            assert_eq!(
                targets.name().parse::<FuzzTargets>().expect("parses"),
                *targets
            );
        }

        let error = "cauchy"
            .parse::<FuzzDistribution>()
            .expect_err("cauchy is not offered");
        // The message must name the alternatives, not just the mistake.
        assert!(error.to_string().contains("gaussian"), "{error}");
        assert!("multiplicative".parse::<FuzzMode>().is_err());
        assert!("everything".parse::<FuzzTargets>().is_err());
    }

    #[test]
    fn defaults_to_perturbing_inputs_only() {
        let shape = RecordShape::new(3, 2).expect("valid shape");

        assert_eq!(FuzzTargets::default(), FuzzTargets::Inputs);
        assert_eq!(
            (0..5)
                .map(|index| FuzzTargets::Inputs.includes(index, &shape))
                .collect::<Vec<_>>(),
            vec![true, true, true, false, false]
        );
        assert_eq!(
            (0..5)
                .map(|index| FuzzTargets::Outputs.includes(index, &shape))
                .collect::<Vec<_>>(),
            vec![false, false, false, true, true]
        );
        assert!((0..5).all(|index| FuzzTargets::All.includes(index, &shape)));
    }

    #[test]
    fn applies_absolute_noise_as_an_offset() {
        let perturbed = policy(FuzzMode::Absolute, FuzzBounds::default()).perturb(10.0, 1.0);

        assert_eq!(perturbed.value, 10.5);
        assert!(!perturbed.clamped);
        assert!(!perturbed.preserved);
    }

    #[test]
    fn applies_relative_noise_in_proportion_to_the_value() {
        let policy = policy(FuzzMode::Relative, FuzzBounds::default());

        assert_eq!(policy.perturb(10.0, 1.0).value, 15.0);
        assert_eq!(policy.perturb(-10.0, 1.0).value, -15.0);
        assert_eq!(
            policy.perturb(0.0, 1.0).value,
            0.0,
            "a relative perturbation leaves an exact zero alone"
        );
    }

    #[test]
    fn reports_a_value_a_bound_moved() {
        let bounds = FuzzBounds::new(Some(-1.0), Some(1.0)).expect("valid bounds");
        let policy = policy(FuzzMode::Absolute, bounds);

        let high = policy.perturb(10.0, 1.0);
        assert_eq!(high.value, 1.0);
        assert!(high.clamped);

        let low = policy.perturb(-10.0, -1.0);
        assert_eq!(low.value, -1.0);
        assert!(low.clamped);

        let inside = policy.perturb(0.0, 1.0);
        assert_eq!(inside.value, 0.5);
        assert!(!inside.clamped);
    }

    #[test]
    fn preserves_a_non_finite_source_value() {
        let policy = policy(FuzzMode::Absolute, FuzzBounds::default());

        for value in [f32::INFINITY, f32::NEG_INFINITY] {
            let perturbed = policy.perturb(value, 1.0);
            assert_eq!(perturbed.value, value);
            assert!(perturbed.preserved);
            assert!(!perturbed.clamped);
        }
        assert!(policy.perturb(f32::NAN, 1.0).value.is_nan());
        assert!(policy.perturb(f32::NAN, 1.0).preserved);
    }

    #[test]
    fn leaves_an_unrepresentable_result_non_finite_for_the_run_to_refuse() {
        let bounds = FuzzBounds::new(None, Some(1.0)).expect("valid bounds");
        let policy = FuzzPolicy::new(
            FuzzDistribution::Uniform,
            1.0,
            FuzzMode::Relative,
            FuzzTargets::Inputs,
            bounds,
        )
        .expect("a valid policy");

        let perturbed = policy.perturb(f32::MAX, 1.0);

        assert!(
            perturbed.value.is_infinite(),
            "a bound must not disguise an overflow as a value"
        );
        assert!(!perturbed.preserved);
    }

    #[test]
    fn records_the_whole_policy_including_absent_bounds() {
        let parameters = policy(FuzzMode::Absolute, FuzzBounds::default()).parameters();

        assert_eq!(parameters["distribution"], "gaussian");
        assert_eq!(parameters["scale"], 0.5);
        assert_eq!(parameters["mode"], "absolute");
        assert_eq!(parameters["targets"], "inputs");
        assert!(parameters["clamp_min"].is_null());
        assert!(parameters["clamp_max"].is_null());
        assert_eq!(parameters["non_finite_source"], "preserve");
        assert_eq!(parameters["non_finite_result"], "fail");
    }

    #[test]
    fn rejects_a_scale_that_perturbs_nothing_or_cannot_be_applied() {
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let error = FuzzPolicy::new(
                FuzzDistribution::Gaussian,
                scale,
                FuzzMode::Absolute,
                FuzzTargets::Inputs,
                FuzzBounds::default(),
            )
            .expect_err("the scale is validated");

            assert!(matches!(error, FuzzError::InvalidScale { .. }), "{error:?}");
        }
    }

    #[test]
    fn rejects_bounds_that_cannot_hold_a_value() {
        assert!(FuzzBounds::new(Some(1.0), Some(-1.0)).is_err());
        assert!(FuzzBounds::new(Some(f32::NAN), None).is_err());
        assert!(FuzzBounds::new(None, Some(f32::NEG_INFINITY)).is_err());

        let equal = FuzzBounds::new(Some(2.0), Some(2.0)).expect("equal bounds are allowed");
        assert_eq!(equal.min(), Some(2.0));
        assert_eq!(equal.max(), Some(2.0));
        assert_eq!(FuzzBounds::default(), FuzzBounds::new(None, None).unwrap());
    }
}
