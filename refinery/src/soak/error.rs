//! Failures of a soak run.
//!
//! A soak exists to produce trustworthy evidence, so every variant here ends
//! the run. Nothing is downgraded to a warning inside a report that would
//! otherwise read as a clean result.

use std::error::Error;
use std::fmt;
use std::io;
use std::path::PathBuf;

use crate::corpus::CorpusError;
use crate::manifest::ManifestError;

/// A soak run that could not be completed, or whose evidence did not hold.
#[derive(Debug)]
#[non_exhaustive]
pub enum SoakError {
    /// A filesystem operation the soak depends on failed.
    Io {
        /// The path being operated on.
        path: PathBuf,
        /// The originating error.
        source: io::Error,
    },
    /// A measured process could not be started at all.
    Spawn {
        /// The program that could not be spawned.
        program: String,
        /// The originating error.
        source: io::Error,
    },
    /// A measured process ran and failed.
    CommandFailed {
        /// The label the soak measures that process under.
        label: String,
        /// Its exit code, when it exited normally.
        code: Option<i32>,
        /// What it wrote to standard error.
        stderr: String,
    },
    /// An invariant the cut-over depends on did not hold.
    Invariant {
        /// Which check failed.
        check: &'static str,
        /// What was observed.
        detail: String,
    },
    /// A corpus could not be read or discovered.
    Corpus {
        /// The originating error.
        source: CorpusError,
    },
    /// A published manifest could not be read or verified.
    Manifest {
        /// The originating error.
        source: ManifestError,
    },
    /// The report could not be encoded as JSON.
    Json {
        /// The originating error.
        source: serde_json::Error,
    },
}

impl SoakError {
    /// Wraps `source` with the `path` it applies to.
    pub(crate) fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    /// Reports a broken invariant, naming the check and what was seen.
    pub(crate) fn invariant(check: &'static str, detail: impl Into<String>) -> Self {
        Self::Invariant {
            check,
            detail: detail.into(),
        }
    }
}

impl From<CorpusError> for SoakError {
    fn from(source: CorpusError) -> Self {
        Self::Corpus { source }
    }
}

impl From<ManifestError> for SoakError {
    fn from(source: ManifestError) -> Self {
        Self::Manifest { source }
    }
}

impl From<serde_json::Error> for SoakError {
    fn from(source: serde_json::Error) -> Self {
        Self::Json { source }
    }
}

impl fmt::Display for SoakError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "soak run {}: {source}", path.display()),
            Self::Spawn { program, source } => {
                write!(f, "the soak could not start {program}: {source}")
            }
            Self::CommandFailed {
                label,
                code,
                stderr,
            } => write!(
                f,
                "the soak run {label} exited {} — {}",
                code.map_or_else(|| "on a signal".to_string(), |code| code.to_string()),
                stderr.trim()
            ),
            Self::Invariant { check, detail } => {
                write!(f, "soak invariant {check} did not hold: {detail}")
            }
            Self::Corpus { source } => write!(f, "soak corpus: {source}"),
            Self::Manifest { source } => write!(f, "soak provenance: {source}"),
            Self::Json { source } => write!(f, "the soak report is not encodable: {source}"),
        }
    }
}

impl Error for SoakError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } | Self::Spawn { source, .. } => Some(source),
            Self::Corpus { source } => Some(source),
            Self::Manifest { source } => Some(source),
            Self::Json { source } => Some(source),
            Self::CommandFailed { .. } | Self::Invariant { .. } => None,
        }
    }
}
