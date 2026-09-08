//! Failures of a sampling run.
//!
//! Every variant is fatal: a sample that cannot be produced exactly is never
//! published as a partial or approximate one.

use std::error::Error;
use std::fmt;
use std::io;
use std::path::PathBuf;

use crate::corpus::CorpusError;
use crate::manifest::ManifestError;
use crate::transform::TransformError;

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
    /// The provenance record could not be produced, so nothing was published.
    Manifest(ManifestError),
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
    /// The target volume filled up, carrying what a whole fresh attempt costs.
    ///
    /// The run itself failed as one of the variants above; this wrapper adds
    /// the one fact a caller deciding whether to retry cannot work out for
    /// itself — how much free space the next attempt needs.
    StorageFull {
        /// Bytes a whole fresh pass publishes, headroom included, and never
        /// fewer than the failed attempt had already written.
        required_bytes: u64,
        /// The out-of-space failure itself.
        source: Box<SampleError>,
    },
}

impl SampleError {
    /// This failure restated as a full volume needing `required_bytes`, or
    /// unchanged when the run did not stop for want of space.
    ///
    /// Only an out-of-space failure is wrapped: every other failure repeats on
    /// a retry, so reporting a space requirement beside it would invite a
    /// caller to free space and try again for nothing.
    #[must_use]
    pub fn with_required_bytes(self, required_bytes: u64) -> Self {
        if self.required_bytes().is_some() || !crate::exit::is_storage_full(&self) {
            return self;
        }
        Self::StorageFull {
            required_bytes,
            source: Box::new(self),
        }
    }

    /// Bytes a whole fresh attempt needs, when this failure knows.
    ///
    /// `None` is the honest answer for every failure that carries no estimate:
    /// a caller gating a retry on the figure refuses rather than guessing.
    #[must_use]
    pub const fn required_bytes(&self) -> Option<u64> {
        match self {
            Self::StorageFull { required_bytes, .. } => Some(*required_bytes),
            _ => None,
        }
    }
}

impl From<CorpusError> for SampleError {
    fn from(error: CorpusError) -> Self {
        Self::Corpus(error)
    }
}

impl From<ManifestError> for SampleError {
    fn from(error: ManifestError) -> Self {
        Self::Manifest(error)
    }
}

impl From<TransformError> for SampleError {
    /// Restates a shared transform failure in the sampler's own vocabulary, so
    /// a caller matching on [`SampleError`] sees one error type rather than two.
    fn from(error: TransformError) -> Self {
        match error {
            TransformError::NoCorpusFiles { path } => Self::NoCorpusFiles { path },
            TransformError::OverlappingCorpora { output, source } => {
                Self::OverlappingCorpora { output, source }
            }
            TransformError::Corpus(error) => Self::Corpus(error),
            TransformError::Manifest(error) => Self::Manifest(error),
            TransformError::Publish {
                staging,
                destination,
                source,
            } => Self::Publish {
                staging,
                destination,
                source,
            },
            TransformError::Io { path, source } => Self::Io { path, source },
        }
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
            // Metadata is validated before any file is opened; every other
            // manifest failure happens with a corpus staged and unpublished,
            // and the operator needs to be told it was thrown away.
            Self::Manifest(error @ ManifestError::InvalidMetadata { .. }) => write!(f, "{error}"),
            Self::Manifest(error) => write!(
                f,
                "{error} — nothing was published: a derived corpus is never published without its provenance"
            ),
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
            // The machine-readable figure is a line of its own — see
            // [`crate::exit::failure_report`] — so this says the same thing in
            // the operator's words rather than repeating the token.
            Self::StorageFull {
                required_bytes,
                source,
            } => write!(
                f,
                "{source} — a whole fresh attempt needs {required_bytes} bytes of free space"
            ),
        }
    }
}

impl Error for SampleError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Corpus(error) => Some(error),
            Self::Manifest(error) => Some(error),
            Self::Publish { source, .. } | Self::Io { source, .. } => Some(source),
            // The wrapped failure is what the volume actually raised, so the
            // out-of-space classification still reads it off the chain.
            Self::StorageFull { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}
