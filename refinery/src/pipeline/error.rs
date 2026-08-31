//! Failures of a pipeline run.
//!
//! Every variant is fatal: a pipeline that cannot complete every stage exactly
//! publishes nothing, and the previously published corpus is left as it was. A
//! stage failure keeps the stage's own error as its source, so the operator
//! reads the transform's explanation with the position it happened at in front
//! of it.

use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::corpus::CorpusError;
use crate::fuzz::FuzzError;
use crate::manifest::ManifestError;
use crate::quantise::QuantiseError;
use crate::sample::SampleError;
use crate::transform::TransformError;

/// A pipeline run that could not be completed.
#[derive(Debug)]
#[non_exhaustive]
pub enum PipelineError {
    /// The configuration listed no stages, so there is nothing to run.
    EmptyPipeline,
    /// The configuration carries a schema version this build does not know.
    UnsupportedConfigVersion {
        /// The version the configuration declared.
        found: u32,
        /// The version this build reads.
        expected: u32,
    },
    /// One stage could not be planned or could not be run.
    Stage {
        /// Its position in the pipeline, counting from one.
        position: usize,
        /// The transform it names.
        name: String,
        /// Why it failed.
        source: Box<StageError>,
    },
    /// The shared transform machinery failed — discovery, separation, staging
    /// or publication.
    Transform(TransformError),
    /// The configuration is not valid JSON, or not a pipeline configuration.
    Json {
        /// The configuration being read.
        path: PathBuf,
        /// The originating error.
        source: serde_json::Error,
    },
    /// An underlying filesystem operation failed.
    Io {
        /// The path being operated on.
        path: PathBuf,
        /// The originating error.
        source: io::Error,
    },
}

impl PipelineError {
    /// Wraps `source` with the `path` it applies to.
    pub(crate) fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    /// Wraps a configuration decoding failure with the file it came from.
    pub(crate) fn json(path: &Path, source: serde_json::Error) -> Self {
        Self::Json {
            path: path.to_path_buf(),
            source,
        }
    }

    /// Wraps `source` with the stage that produced it.
    pub(crate) fn stage(position: usize, name: &str, source: StageError) -> Self {
        Self::Stage {
            position,
            name: name.to_string(),
            source: Box::new(source),
        }
    }
}

impl From<TransformError> for PipelineError {
    fn from(error: TransformError) -> Self {
        Self::Transform(error)
    }
}

impl From<CorpusError> for PipelineError {
    fn from(error: CorpusError) -> Self {
        Self::Transform(error.into())
    }
}

impl From<ManifestError> for PipelineError {
    fn from(error: ManifestError) -> Self {
        Self::Transform(error.into())
    }
}

impl fmt::Display for PipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPipeline => write!(
                f,
                "the pipeline lists no stages — a pipeline states the transforms to apply, in order"
            ),
            Self::UnsupportedConfigVersion { found, expected } => write!(
                f,
                "pipeline configuration version {found} is not one this build reads (expected {expected})"
            ),
            Self::Stage {
                position,
                name,
                source,
            } => write!(f, "pipeline stage {position} ({name}): {source}"),
            Self::Transform(error) => write!(f, "{error}"),
            Self::Json { path, source } => write!(
                f,
                "{} is not a pipeline configuration: {source}",
                path.display()
            ),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl Error for PipelineError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Stage { source, .. } => Some(source),
            Self::Transform(error) => Some(error),
            Self::Json { source, .. } => Some(source),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Why one stage of a pipeline failed.
///
/// A stage is the ordinary standalone transform, so its failure is the
/// transform's own — reported verbatim rather than flattened into a string.
#[derive(Debug)]
#[non_exhaustive]
pub enum StageError {
    /// A `sample` stage failed.
    Sample(SampleError),
    /// A `fuzz` stage failed.
    Fuzz(FuzzError),
    /// A `quantise` stage failed.
    Quantise(QuantiseError),
}

impl fmt::Display for StageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sample(error) => write!(f, "{error}"),
            Self::Fuzz(error) => write!(f, "{error}"),
            Self::Quantise(error) => write!(f, "{error}"),
        }
    }
}

impl Error for StageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Sample(error) => Some(error),
            Self::Fuzz(error) => Some(error),
            Self::Quantise(error) => Some(error),
        }
    }
}

impl From<SampleError> for StageError {
    fn from(error: SampleError) -> Self {
        Self::Sample(error)
    }
}

impl From<FuzzError> for StageError {
    fn from(error: FuzzError) -> Self {
        Self::Fuzz(error)
    }
}

impl From<QuantiseError> for StageError {
    fn from(error: QuantiseError) -> Self {
        Self::Quantise(error)
    }
}
