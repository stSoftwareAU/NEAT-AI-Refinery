//! Failures of a quantisation run.
//!
//! Every variant is fatal. A corpus that cannot be re-encoded exactly as the
//! named scheme describes is never published approximately.

use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use super::QuantiseScheme;
use crate::corpus::CorpusError;
use crate::manifest::ManifestError;
use crate::transform::TransformError;

/// A quantisation run that could not be completed.
#[derive(Debug)]
#[non_exhaustive]
pub enum QuantiseError {
    /// The requested scheme is not one Refinery offers.
    UnknownScheme {
        /// The scheme name as supplied.
        scheme: String,
    },
    /// The source corpus is not encoded the way the scheme expects to read it.
    ///
    /// Raised from the source's own manifest, so quantising an already
    /// quantised corpus fails loud instead of reinterpreting its bytes.
    SourceEncodingMismatch {
        /// The manifest that was consulted.
        manifest: PathBuf,
        /// The encoding the scheme reads.
        expected: String,
        /// The encoding the source manifest declares.
        found: String,
    },
    /// The source corpus declares a record width the caller's shape disagrees
    /// with, so the records would be split in the wrong places.
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

impl From<TransformError> for QuantiseError {
    fn from(error: TransformError) -> Self {
        Self::Transform(error)
    }
}

impl From<CorpusError> for QuantiseError {
    fn from(error: CorpusError) -> Self {
        Self::Transform(TransformError::Corpus(error))
    }
}

impl From<ManifestError> for QuantiseError {
    fn from(error: ManifestError) -> Self {
        Self::Transform(TransformError::Manifest(error))
    }
}

impl fmt::Display for QuantiseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownScheme { scheme } => {
                let offered: Vec<&str> = QuantiseScheme::ALL
                    .iter()
                    .map(|scheme| scheme.name())
                    .collect();
                write!(
                    f,
                    "unknown quantisation scheme {scheme:?} — Refinery offers: {}",
                    offered.join(", ")
                )
            }
            Self::SourceEncodingMismatch {
                manifest,
                expected,
                found,
            } => write!(
                f,
                "source corpus is encoded as {found}, but the scheme reads {expected} — {} says so",
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

impl Error for QuantiseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Transform(error) => Some(error),
            _ => None,
        }
    }
}
