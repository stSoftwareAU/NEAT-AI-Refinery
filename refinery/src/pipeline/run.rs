//! The pipeline run itself.
//!
//! Each stage is the ordinary standalone transform, run over the previous
//! stage's published output inside a scratch tree. Only the last stage's corpus
//! is published, with a manifest recording every stage in order.

use std::fs;
use std::path::{Path, PathBuf};

use rand::RngExt;

use super::{PipelineError, PipelineRequest, PlannedStage, StageError, StageKind};
use crate::corpus::RecordShape;
use crate::fuzz::{fuzz, FuzzRequest};
use crate::manifest::{
    CallerMetadata, Manifest, RecordGeometry, SourceIdentity, TransformRecord, MANIFEST_FILE_NAME,
};
use crate::quantise::{quantise, QuantiseRequest};
use crate::sample::{sample, SampleRequest};
use crate::transform::{resolved_source, StagedCorpus};

/// The transform name a pipeline manifest records.
const TRANSFORM_NAME: &str = "pipeline";

/// What a completed pipeline run produced.
#[derive(Debug, Clone)]
pub struct PipelineOutcome {
    /// Every stage that ran, in the order it ran.
    pub stages: Vec<StageOutcome>,
    /// Records the first stage read from the source corpus.
    pub records_read: u64,
    /// Records the last stage published.
    pub records_written: u64,
    /// The published corpus file.
    pub output_file: PathBuf,
    /// The pipeline seed — supplied, or drawn from the operating system. Every
    /// stage seed follows from it, so this one value replays the whole run.
    pub seed: u64,
    /// The published manifest file.
    pub manifest_file: PathBuf,
    /// The provenance record published beside the corpus.
    pub manifest: Manifest,
}

/// What one stage of a pipeline produced.
#[derive(Debug, Clone)]
pub struct StageOutcome {
    /// Its position in the pipeline, counting from one.
    pub position: usize,
    /// The transform, its parameters and the seed it ran under — exactly as
    /// the stage's own manifest recorded them.
    pub transform: TransformRecord,
    /// Records the stage read.
    pub records_read: u64,
    /// Records the stage wrote.
    pub records_written: u64,
}

/// What one stage left behind for the next one.
struct StageRun {
    /// The stage's own manifest.
    manifest: Manifest,
    /// The record layout the next stage reads.
    shape: RecordShape,
    /// Records read and written.
    records_read: u64,
    records_written: u64,
}

/// Applies every configured stage in order and publishes the final corpus.
///
/// The source is only ever read. Intermediate corpora are built inside a
/// staging directory beside the destination and removed when the run ends, so
/// a failed pipeline publishes nothing and leaves no scratch behind, and the
/// final corpus is swapped in with a single atomic rename.
///
/// # Errors
///
/// Returns [`PipelineError::EmptyPipeline`] for a configuration with no
/// stages, [`PipelineError::UnsupportedConfigVersion`] for a schema this build
/// does not read, [`PipelineError::Stage`] when a stage's parameters are
/// refused or its run fails, [`PipelineError::Transform`] when the source
/// cannot be resolved or the result cannot be staged or published, and
/// [`PipelineError::Io`] for any other filesystem failure.
pub fn run_pipeline(request: &PipelineRequest) -> Result<PipelineOutcome, PipelineError> {
    request.config.validate()?;
    // Publishing replaces the whole output directory, so an output overlapping
    // the immutable source is refused before anything is read or created.
    resolved_source(&request.source, &request.output)?;

    let seed = request.config.seed.unwrap_or_else(|| rand::rng().random());
    let planned = request.config.plan(seed)?;

    // One scratch tree for the whole run: every intermediate corpus lives
    // inside it, and it is removed whether the run succeeds or fails.
    let staged = StagedCorpus::create(&request.output)?;

    let mut shape = request.shape;
    let mut current = request.source.clone();
    let mut stage_dirs: Vec<PathBuf> = Vec::with_capacity(planned.len());
    let mut stages: Vec<StageOutcome> = Vec::with_capacity(planned.len());
    let mut source_identity: Option<SourceIdentity> = None;
    let mut last: Option<Manifest> = None;

    for stage in &planned {
        let directory = staged.path().join(stage.directory_name());
        let run = run_stage(stage, &current, &directory, shape, &request.metadata)?;

        source_identity.get_or_insert_with(|| run.manifest.source.clone());
        stages.push(StageOutcome {
            position: stage.position,
            transform: run.manifest.transform.clone(),
            records_read: run.records_read,
            records_written: run.records_written,
        });
        shape = run.shape;
        current.clone_from(&directory);
        stage_dirs.push(directory);
        last = Some(run.manifest);
    }

    // `validate` refused an empty pipeline, so at least one stage ran.
    let last = last.ok_or(PipelineError::EmptyPipeline)?;
    let source = source_identity.ok_or(PipelineError::EmptyPipeline)?;

    // The last stage's corpus becomes the pipeline's, unchanged: it is moved
    // out of its scratch directory rather than rewritten, so the record count,
    // byte length and checksum its manifest recorded still describe it.
    let corpus = last.output.file.clone();
    let published_corpus = staged.path().join(&corpus);
    let staged_corpus = stage_dirs
        .last()
        .ok_or(PipelineError::EmptyPipeline)?
        .join(&corpus);
    fs::rename(&staged_corpus, &published_corpus)
        .map_err(|e| PipelineError::io(&published_corpus, e))?;
    for directory in &stage_dirs {
        fs::remove_dir_all(directory).map_err(|e| PipelineError::io(directory, e))?;
    }

    let published_shape: RecordGeometry = shape.into();
    let source_shape: RecordGeometry = request.shape.into();
    let mut manifest = Manifest::new(
        TransformRecord::new(TRANSFORM_NAME, parameters(&planned), Some(seed)),
        published_shape.clone(),
        source,
        last.output.clone(),
        request.metadata.clone(),
    )
    .with_pipeline(
        stages
            .iter()
            .map(|stage| stage.transform.clone())
            .collect::<Vec<_>>(),
    );
    // Recorded on the same rule a single transform follows: an absent source
    // layout means both corpora share one.
    if published_shape != source_shape {
        manifest = manifest.with_source_record_shape(source_shape);
    }
    manifest.write_into(staged.path())?;

    let output_file = staged.destination().join(&corpus);
    let manifest_file = staged.destination().join(MANIFEST_FILE_NAME);
    staged.publish()?;

    Ok(PipelineOutcome {
        records_read: stages.first().map_or(0, |stage| stage.records_read),
        records_written: stages.last().map_or(0, |stage| stage.records_written),
        stages,
        output_file,
        seed,
        manifest_file,
        manifest,
    })
}

/// The parameters the manifest records for the pipeline itself.
///
/// The stages are recorded in full beside it, so only what describes the run as
/// a whole belongs here.
fn parameters(planned: &[PlannedStage]) -> std::collections::BTreeMap<String, serde_json::Value> {
    let mut parameters = std::collections::BTreeMap::new();
    parameters.insert(
        "config_version".to_string(),
        serde_json::Value::from(super::PIPELINE_CONFIG_VERSION),
    );
    parameters.insert(
        "stage_count".to_string(),
        serde_json::Value::from(planned.len()),
    );
    parameters
}

/// Runs one stage as the standalone transform it is, publishing into
/// `output` — a directory inside the run's scratch tree.
fn run_stage(
    stage: &PlannedStage,
    source: &Path,
    output: &Path,
    shape: RecordShape,
    metadata: &CallerMetadata,
) -> Result<StageRun, PipelineError> {
    let failed = |error: StageError| PipelineError::stage(stage.position, stage.name(), error);

    match &stage.kind {
        StageKind::Sample(rate) => {
            let outcome = sample(&SampleRequest {
                source: source.to_path_buf(),
                output: output.to_path_buf(),
                shape,
                rate: *rate,
                seed: stage.seed,
                metadata: metadata.clone(),
            })
            .map_err(|error| failed(error.into()))?;
            Ok(StageRun {
                manifest: outcome.manifest,
                shape,
                records_read: outcome.records_read,
                records_written: outcome.records_written,
            })
        }
        StageKind::Fuzz(policy) => {
            let outcome = fuzz(&FuzzRequest {
                source: source.to_path_buf(),
                output: output.to_path_buf(),
                shape,
                policy: *policy,
                seed: stage.seed,
                metadata: metadata.clone(),
            })
            .map_err(|error| failed(error.into()))?;
            Ok(StageRun {
                manifest: outcome.manifest,
                shape,
                records_read: outcome.records_read,
                records_written: outcome.records_written,
            })
        }
        StageKind::Quantise(scheme) => {
            let request = QuantiseRequest {
                source: source.to_path_buf(),
                output: output.to_path_buf(),
                shape,
                scheme: *scheme,
                metadata: metadata.clone(),
            };
            // Quantisation is the one stage that changes the layout: the next
            // stage reads what this one published, not what it read.
            let target = request
                .target_shape()
                .map_err(|error| failed(error.into()))?;
            let outcome = quantise(&request).map_err(|error| failed(error.into()))?;
            Ok(StageRun {
                manifest: outcome.manifest,
                shape: target,
                records_read: outcome.records_read,
                records_written: outcome.records_written,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{PipelineConfig, PipelineStage, QuantiseStage, SampleStage};

    #[test]
    fn records_the_schema_version_and_stage_count() {
        let planned = PipelineConfig::new(vec![
            PipelineStage::Sample(SampleStage {
                rate: 1.0,
                seed: None,
            }),
            PipelineStage::Quantise(QuantiseStage {
                scheme: "bfloat16".to_string(),
            }),
        ])
        .plan(3)
        .expect("valid stages");

        let parameters = parameters(&planned);

        assert_eq!(parameters["config_version"], 1);
        assert_eq!(parameters["stage_count"], 2);
    }
}
