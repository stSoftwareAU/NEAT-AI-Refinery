//! Failures of the provenance record.
//!
//! Every variant is fatal to the run that raised it: a derived corpus is never
//! published with provenance that could not be written faithfully.

use std::error::Error;
use std::fmt;
use std::io;
use std::path::PathBuf;

/// A manifest that could not be produced, written or read back.
#[derive(Debug)]
#[non_exhaustive]
pub enum ManifestError {
    /// Caller metadata that cannot be recorded as an opaque key/value pair.
    InvalidMetadata {
        /// The entry as supplied.
        entry: String,
        /// Why it was refused.
        reason: &'static str,
    },
    /// The manifest could not be encoded as JSON, or a stored one could not be
    /// decoded.
    Json {
        /// The originating error.
        source: serde_json::Error,
    },
    /// A filesystem operation on the manifest or the corpus it describes failed.
    Io {
        /// The path being operated on.
        path: PathBuf,
        /// The originating error.
        source: io::Error,
    },
}

impl ManifestError {
    /// Wraps `source` with the `path` it applies to.
    pub(crate) fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    /// Refuses a caller metadata `entry`, saying why.
    pub(crate) fn invalid_metadata(entry: impl Into<String>, reason: &'static str) -> Self {
        Self::InvalidMetadata {
            entry: entry.into(),
            reason,
        }
    }
}

impl From<serde_json::Error> for ManifestError {
    fn from(source: serde_json::Error) -> Self {
        Self::Json { source }
    }
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMetadata { entry, reason } => {
                write!(f, "invalid caller metadata {entry:?}: {reason}")
            }
            Self::Json { source } => write!(f, "the transformation manifest is not valid JSON: {source}"),
            Self::Io { path, source } => {
                write!(f, "transformation manifest {}: {source}", path.display())
            }
        }
    }
}

impl Error for ManifestError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Json { source } => Some(source),
            Self::Io { source, .. } => Some(source),
            Self::InvalidMetadata { .. } => None,
        }
    }
}
