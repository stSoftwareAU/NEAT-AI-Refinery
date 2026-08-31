//! Failures of a benchmark run.
//!
//! Every variant ends the run. A benchmark exists to produce numbers somebody
//! will act on, so a measurement that could not be taken, an invariant that
//! did not hold, and a regression against the gate are all errors — never a
//! footnote inside a report that still reads as a pass.

use std::error::Error;
use std::fmt;
use std::io;
use std::path::PathBuf;

use crate::corpus::CorpusError;
use crate::manifest::ManifestError;
use crate::pipeline::PipelineError;
use crate::soak::SoakError;

/// A benchmark that could not be run, or whose result did not clear its gate.
#[derive(Debug)]
#[non_exhaustive]
pub enum BenchError {
    /// A filesystem operation the benchmark depends on failed.
    Io {
        /// The path being operated on.
        path: PathBuf,
        /// The originating error.
        source: io::Error,
    },
    /// A measured process could not be started, or ran and failed.
    Measure {
        /// The originating error.
        source: SoakError,
    },
    /// The synthetic corpus could not be written.
    Corpus {
        /// The originating error.
        source: CorpusError,
    },
    /// A published manifest could not be read.
    Manifest {
        /// The originating error.
        source: ManifestError,
    },
    /// A pipeline case could not be described to the binary.
    Pipeline {
        /// The originating error.
        source: PipelineError,
    },
    /// The report could not be encoded or decoded as JSON.
    Json {
        /// The originating error.
        source: serde_json::Error,
    },
    /// The benchmark was asked for something it cannot measure or compare.
    Config {
        /// What was asked for, and why it cannot be honoured.
        detail: String,
    },
    /// A measured run breached an invariant the numbers depend on.
    Invariant {
        /// Which check failed.
        check: &'static str,
        /// What was observed.
        detail: String,
    },
    /// A run did not clear the gate it was held to.
    Regression {
        /// What regressed, by how much, and against what.
        detail: String,
    },
}

impl BenchError {
    /// Wraps `source` with the `path` it applies to.
    pub(crate) fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    /// Reports a request the benchmark cannot honour.
    pub(crate) fn config(detail: impl Into<String>) -> Self {
        Self::Config {
            detail: detail.into(),
        }
    }

    /// Reports a broken invariant, naming the check and what was seen.
    pub(crate) fn invariant(check: &'static str, detail: impl Into<String>) -> Self {
        Self::Invariant {
            check,
            detail: detail.into(),
        }
    }

    /// Reports a run that did not clear its gate.
    pub(crate) fn regression(detail: impl Into<String>) -> Self {
        Self::Regression {
            detail: detail.into(),
        }
    }
}

impl From<SoakError> for BenchError {
    fn from(source: SoakError) -> Self {
        Self::Measure { source }
    }
}

impl From<CorpusError> for BenchError {
    fn from(source: CorpusError) -> Self {
        Self::Corpus { source }
    }
}

impl From<ManifestError> for BenchError {
    fn from(source: ManifestError) -> Self {
        Self::Manifest { source }
    }
}

impl From<PipelineError> for BenchError {
    fn from(source: PipelineError) -> Self {
        Self::Pipeline { source }
    }
}

impl From<serde_json::Error> for BenchError {
    fn from(source: serde_json::Error) -> Self {
        Self::Json { source }
    }
}

impl fmt::Display for BenchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "benchmark run {}: {source}", path.display()),
            Self::Measure { source } => write!(f, "benchmark measurement: {source}"),
            Self::Corpus { source } => write!(f, "benchmark corpus: {source}"),
            Self::Manifest { source } => write!(f, "benchmark provenance: {source}"),
            Self::Pipeline { source } => write!(f, "benchmark pipeline case: {source}"),
            Self::Json { source } => write!(f, "the benchmark report is not encodable: {source}"),
            Self::Config { detail } => write!(f, "the benchmark cannot be run as asked: {detail}"),
            Self::Invariant { check, detail } => {
                write!(f, "benchmark invariant {check} did not hold: {detail}")
            }
            Self::Regression { detail } => write!(f, "performance regression: {detail}"),
        }
    }
}

impl Error for BenchError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Measure { source } => Some(source),
            Self::Corpus { source } => Some(source),
            Self::Manifest { source } => Some(source),
            Self::Pipeline { source } => Some(source),
            Self::Json { source } => Some(source),
            Self::Config { .. } | Self::Invariant { .. } | Self::Regression { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_regression_says_what_regressed() {
        let error = BenchError::regression("sample fell to 0.40× the baseline");

        assert!(error.to_string().contains("performance regression"));
        assert!(error.to_string().contains("0.40×"));
    }

    #[test]
    fn an_unmeasurable_request_names_itself() {
        let error = BenchError::config("a benchmark of zero repeats measures nothing");

        assert!(error.to_string().contains("zero repeats"));
        assert!(error.source().is_none());
    }

    #[test]
    fn a_measurement_failure_keeps_the_cause() {
        let error = BenchError::from(SoakError::invariant("published bytes", "detail"));

        assert!(error.to_string().contains("benchmark measurement"));
        assert!(error.source().is_some(), "the cause must survive wrapping");
    }
}
