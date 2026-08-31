//! What a benchmark measures: one labelled transform, run over the corpus.
//!
//! A case is deliberately thin — a label and the transform the binary is asked
//! to run — because the point of the harness is that every case is measured
//! the same way. The label is how a case is matched to its baseline, so it is
//! stable and short.

use crate::pipeline::{PipelineConfig, PipelineStage, QuantiseStage, SampleStage};

/// The transform one benchmark case runs.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum BenchTransform {
    /// Materialised sampling at a rate.
    Sample {
        /// Probability each record is kept, in `(0, 1]`.
        rate: f64,
    },
    /// Re-encoding every value under a narrower scheme.
    Quantise {
        /// The scheme — `bfloat16`.
        scheme: String,
    },
    /// An ordered chain of transforms, published as one corpus.
    Pipeline {
        /// The stages, in the order they are applied.
        config: PipelineConfig,
    },
}

/// One labelled workload in a benchmark suite.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchCase {
    /// The label the case is measured and compared under.
    pub label: String,
    /// The transform it runs.
    pub transform: BenchTransform,
}

impl BenchCase {
    /// A case running `transform` under `label`.
    #[must_use]
    pub fn new(label: impl Into<String>, transform: BenchTransform) -> Self {
        Self {
            label: label.into(),
            transform,
        }
    }

    /// Materialised sampling at `rate`, labelled `sample`.
    #[must_use]
    pub fn sample(rate: f64) -> Self {
        Self::new("sample", BenchTransform::Sample { rate })
    }

    /// `bfloat16` quantisation of the whole corpus, labelled `quantise`.
    #[must_use]
    pub fn quantise() -> Self {
        Self::new(
            "quantise",
            BenchTransform::Quantise {
                scheme: "bfloat16".to_string(),
            },
        )
    }

    /// Sampling at `rate` and then quantising, labelled `pipeline`.
    ///
    /// The chain is the one a caller is most likely to run — draw a sample,
    /// then store it narrower — and it is where a pipeline's cost differs from
    /// the sum of its stages, because the corpus is read once rather than
    /// twice.
    #[must_use]
    pub fn pipeline(rate: f64) -> Self {
        let config = PipelineConfig::new(vec![
            PipelineStage::Sample(SampleStage { rate, seed: None }),
            PipelineStage::Quantise(QuantiseStage {
                scheme: "bfloat16".to_string(),
            }),
        ]);
        Self::new("pipeline", BenchTransform::Pipeline { config })
    }

    /// The suite every committed report and every CI run measures.
    ///
    /// One case a transform: the sampler GRQ calls, the representation
    /// transform that changes the output size most, and the pipeline that
    /// chains them. Adding a case is a deliberate act — a baseline holds a run
    /// to the cases it recorded, so a case that quietly disappears is itself
    /// reported as a regression.
    #[must_use]
    pub fn standard_suite(rate: f64) -> Vec<Self> {
        vec![Self::sample(rate), Self::quantise(), Self::pipeline(rate)]
    }

    /// How the case reads in a report — the command line it stands for.
    #[must_use]
    pub fn description(&self) -> String {
        match &self.transform {
            BenchTransform::Sample { rate } => format!("sample --rate {rate}"),
            BenchTransform::Quantise { scheme } => format!("quantise --scheme {scheme}"),
            BenchTransform::Pipeline { config } => {
                let stages: Vec<&str> = config.stages.iter().map(PipelineStage::name).collect();
                format!("pipeline {}", stages.join(" → "))
            }
        }
    }

    /// The sampling rate this case runs at, when it samples at all.
    ///
    /// The Deno reference samples, so it can only be compared with a case that
    /// samples at the same rate.
    #[must_use]
    pub fn sample_rate(&self) -> Option<f64> {
        match &self.transform {
            BenchTransform::Sample { rate } => Some(*rate),
            BenchTransform::Quantise { .. } | BenchTransform::Pipeline { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_standard_suite_covers_the_rate_and_the_pipeline() {
        let suite = BenchCase::standard_suite(0.05);

        assert_eq!(
            suite
                .iter()
                .map(|case| case.label.as_str())
                .collect::<Vec<_>>(),
            vec!["sample", "quantise", "pipeline"]
        );
        assert_eq!(suite[0].description(), "sample --rate 0.05");
        assert_eq!(suite[1].description(), "quantise --scheme bfloat16");
        assert_eq!(suite[2].description(), "pipeline sample → quantise");
    }

    #[test]
    fn only_a_sampling_case_carries_a_rate_to_compare_on() {
        assert_eq!(BenchCase::sample(0.25).sample_rate(), Some(0.25));
        assert_eq!(BenchCase::quantise().sample_rate(), None);
        assert_eq!(BenchCase::pipeline(0.25).sample_rate(), None);
    }

    #[test]
    fn a_pipeline_case_is_a_valid_pipeline_configuration() {
        let case = BenchCase::pipeline(0.05);

        match &case.transform {
            BenchTransform::Pipeline { config } => {
                config.validate().expect("the case must be runnable");
                assert_eq!(config.stages.len(), 2);
            }
            other => panic!("expected a pipeline case, got {other:?}"),
        }
    }
}
