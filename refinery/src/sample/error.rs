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
    /// A write stopped because the target volume is full, carrying what the
    /// pass still had to write when it did.
    ///
    /// "It failed" cannot tell a caller whether another attempt is worth
    /// spending, and neither can free space on its own: GRQ's retry gate
    /// approved three attempts on a volume with 19 GB free because nothing said
    /// the pass needed about 19 GB
    /// ([stSoftwareAU/GRQ#4611](https://github.com/stSoftwareAU/GRQ/issues/4611)).
    /// Refinery is the only party that knows what is left to write, so the
    /// failure reports it, and the caller compares it with the free space it
    /// measures for itself.
    StorageFull {
        /// Bytes another attempt needs: the **whole** derived corpus this pass
        /// set out to write, at this record width.
        ///
        /// Not the remainder. A partial corpus is never resumed — a run starts
        /// again from the first record, and a caller sweeping scratch between
        /// attempts deletes the partial output first — so the remainder would
        /// understate what the next attempt has to fit by however much was
        /// already written.
        required_bytes: u64,
        /// Records written when the volume filled up.
        records_written: u64,
        /// Records the whole pass expects to write.
        records_expected: u64,
        /// The out-of-space failure itself.
        source: Box<SampleError>,
    },
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
            // `required_bytes=` is the figure a caller's retry gate reads, so
            // the spelling is a wire contract — see `tests/exit_codes.rs`.
            Self::StorageFull {
                required_bytes,
                records_written,
                records_expected,
                source,
            } => write!(
                f,
                "{source} — out of space with {records_written} of about \
                 {records_expected} records written; another attempt writes the corpus \
                 again from the first record: required_bytes={required_bytes}"
            ),
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
        }
    }
}

impl Error for SampleError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Corpus(error) => Some(error),
            // The out-of-space failure itself stays the source, so the exit
            // code still classifies it as a full volume (see [`crate::exit`]).
            Self::StorageFull { source, .. } => Some(source.as_ref()),
            Self::Manifest(error) => Some(error),
            Self::Publish { source, .. } | Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
