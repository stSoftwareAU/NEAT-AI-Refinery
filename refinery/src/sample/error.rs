//! Failures of a sampling run.
//!
//! Every variant is fatal: a sample that cannot be produced exactly is never
//! published as a partial or approximate one.

use std::error::Error;
use std::fmt;
use std::io;
use std::path::PathBuf;

use crate::corpus::CorpusError;

/// A sampling run that could not be completed.
#[derive(Debug)]
#[non_exhaustive]
pub enum SampleError {
    /// The sample rate is outside the allowed `0 < rate <= 1` range.
    InvalidRate {
        /// The rate as supplied.
        rate: f64,
    },
    /// The source directory holds no `.bin` corpus files.
    NoCorpusFiles {
        /// The directory that was scanned.
        path: PathBuf,
    },
    /// The derived corpus and the source corpus overlap on disk.
    OverlappingCorpora {
        /// The rejected output directory.
        output: PathBuf,
        /// The source directory it overlaps.
        source: PathBuf,
    },
    /// The corpus contract was breached while reading or writing records.
    Corpus(CorpusError),
    /// Publishing the staged corpus over the live directory failed.
    Publish {
        /// The staging directory that was to be published.
        staging: PathBuf,
        /// The live directory it was to become.
        destination: PathBuf,
        /// The originating error.
        source: io::Error,
    },
    /// An underlying filesystem operation failed.
    Io {
        /// The path being operated on.
        path: PathBuf,
        /// The originating error.
        source: io::Error,
    },
}

impl SampleError {
    /// Wraps `source` with the `path` it applies to.
    pub(crate) fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

impl From<CorpusError> for SampleError {
    fn from(error: CorpusError) -> Self {
        Self::Corpus(error)
    }
}

impl fmt::Display for SampleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRate { rate } => write!(
                f,
                "invalid sample rate {rate} — the rate must be greater than 0 and at most 1"
            ),
            Self::NoCorpusFiles { path } => write!(
                f,
                "source directory {} holds no .bin corpus files",
                path.display()
            ),
            Self::OverlappingCorpora { output, source } => write!(
                f,
                "derived corpus {} overlaps the source corpus {} — publishing replaces the whole output directory, and sources are immutable",
                output.display(),
                source.display()
            ),
            Self::Corpus(error) => write!(f, "{error}"),
            Self::Publish {
                staging,
                destination,
                source,
            } => write!(
                f,
                "could not publish {} as {}: {source}",
                staging.display(),
                destination.display()
            ),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl Error for SampleError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Corpus(error) => Some(error),
            Self::Publish { source, .. } | Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
