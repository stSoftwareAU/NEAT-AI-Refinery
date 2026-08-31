//! Ordered transform pipelines — several standard transforms chained in one
//! run, with the order stated rather than assumed.
//!
//! Refinery's transforms already compose by being run one after another over
//! each other's output. A pipeline is that same composition written down: a
//! serialisable list of stages, applied first to last, published as a single
//! derived corpus whose manifest records every stage in order.
//!
//! ```text
//! source ──sample──▶ ·scratch· ──fuzz──▶ ·scratch· ──quantise──▶ derived corpus
//! ```
//!
//! # The order is the configuration
//!
//! Transforms do not generally commute. Fuzzing a corpus and then quantising it
//! is not the same corpus as quantising it and then fuzzing it — the first
//! perturbs `f32` values and rounds the result, the second rounds first and
//! perturbs the rounded values. A pipeline therefore never reorders, merges or
//! deduplicates stages: it runs exactly the list it was given, in exactly that
//! order, and records that order in the manifest.
//!
//! # Nothing is baked in
//!
//! A pipeline is a list of the *existing* transforms. It adds no combined mode,
//! no fused fast path and no special case: each stage is the ordinary
//! standalone transform — [`sample`](crate::sample), [`fuzz`](crate::fuzz),
//! [`quantise`](crate::quantise) — run over the previous stage's output, so
//! every one of them stays independently testable and a one-stage pipeline is
//! byte-for-byte the standalone run.
//!
//! # Reproducibility
//!
//! A run has one seed. Each stage that draws randomness gets its own seed
//! derived from it and its position, so no two stages share a draw sequence and
//! moving a stage changes what it draws. A stage may pin its own seed instead.
//! Omit the pipeline seed and one is drawn from the operating system and
//! reported, exactly as the standalone transforms do — the same source, the
//! same configuration and the same seed always replay the same bytes.
//!
//! # Intermediate corpora are scratch
//!
//! Every stage but the last publishes into a staging directory beside the
//! destination, and the whole scratch tree is removed when the run ends —
//! published or failed. Only the final corpus and its manifest are published,
//! atomically, so a reader never sees a half-built pipeline. A stage that fails
//! aborts the run with nothing published and no scratch left behind.
//!
//! ```no_run
//! use neat_ai_refinery::corpus::RecordShape;
//! use neat_ai_refinery::manifest::CallerMetadata;
//! use neat_ai_refinery::pipeline::{run_pipeline, PipelineConfig, PipelineRequest};
//!
//! let config = PipelineConfig::load("pipeline.json")?;
//! let outcome = run_pipeline(&PipelineRequest {
//!     source: "trainData-binary".into(),
//!     output: "trainData-binary-refined".into(),
//!     shape: RecordShape::new(2511, 1)?,
//!     config,
//!     metadata: CallerMetadata::default(),
//! })?;
//! assert_eq!(outcome.stages.len(), outcome.manifest.pipeline.as_ref().map_or(0, Vec::len));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod config;
mod error;
mod plan;
mod run;

pub use config::{
    FuzzStage, PipelineConfig, PipelineStage, QuantiseStage, SampleStage, PIPELINE_CONFIG_VERSION,
};
pub use error::{PipelineError, StageError};
pub use plan::{PipelineRequest, PlannedStage, StageKind};
pub use run::{run_pipeline, PipelineOutcome, StageOutcome};
