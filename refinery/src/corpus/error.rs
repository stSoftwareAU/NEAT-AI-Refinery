//! Failures in the corpus contract.
//!
//! Every variant is fatal by design: a corpus that cannot be interpreted
//! exactly must fail loud rather than be processed approximately.

use std::error::Error;
use std::fmt;
use std::io;
use std::path::PathBuf;

/// A breach of the fixed-width corpus or immutable-source contract.
#[derive(Debug)]
#[non_exhaustive]
pub enum CorpusError {
    /// A record must carry at least one input value and one output value.
    InvalidRecordShape {
        /// Input values per record, as supplied.
        inputs: usize,
        /// Output values per record, as supplied.
        outputs: usize,
    },
    /// The record width does not fit in a `usize`.
    RecordWidthOverflow {
        /// Input values per record, as supplied.
        inputs: usize,
        /// Output values per record, as supplied.
        outputs: usize,
    },
    /// The source holds no records at all.
    EmptySource {
        /// The source that was opened.
        path: PathBuf,
    },
    /// The source ends mid-record, so its final record cannot be interpreted.
    PartialRecord {
        /// The source that was opened.
        path: PathBuf,
        /// Total size of the source in bytes.
        byte_len: u64,
        /// Bytes each whole record occupies.
        bytes_per_record: u64,
        /// Bytes left over after the last whole record.
        trailing_bytes: u64,
    },
    /// A record was requested beyond the end of the corpus.
    RecordIndexOutOfRange {
        /// The requested record index.
        index: u64,
        /// Records the corpus actually holds.
        record_count: u64,
    },
    /// A stream was opened over no sources at all.
    EmptySourceList,
    /// A record offered to a writer is not exactly one record wide.
    RecordLengthMismatch {
        /// The destination being written.
        path: PathBuf,
        /// Bytes a whole record occupies.
        bytes_per_record: usize,
        /// Bytes actually offered.
        actual: usize,
    },
    /// A source path is neither a regular file nor a directory.
    UnsupportedSourceKind {
        /// The offending path.
        path: PathBuf,
    },
    /// A source directory contains no readable corpus files.
    NoSources {
        /// The directory that was scanned.
        path: PathBuf,
    },
    /// The derived destination resolves to one of the sources.
    DestinationIsSource {
        /// The destination that was rejected.
        path: PathBuf,
    },
    /// The derived destination has no file name to write to.
    InvalidDestination {
        /// The destination that was rejected.
        path: PathBuf,
    },
    /// An underlying filesystem operation failed.
    Io {
        /// The path being operated on.
        path: PathBuf,
        /// The originating error.
        source: io::Error,
    },
}

impl CorpusError {
    /// Wraps `source` with the `path` it applies to.
    pub(crate) fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

impl fmt::Display for CorpusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRecordShape { inputs, outputs } => write!(
                f,
                "invalid record shape: inputs={inputs}, outputs={outputs} — each record needs at least one input and one output value"
            ),
            Self::RecordWidthOverflow { inputs, outputs } => write!(
                f,
                "record width overflows usize: inputs={inputs}, outputs={outputs}"
            ),
            Self::EmptySource { path } => {
                write!(f, "source corpus {} holds no records", path.display())
            }
            Self::PartialRecord {
                path,
                byte_len,
                bytes_per_record,
                trailing_bytes,
            } => write!(
                f,
                "source corpus {} ends mid-record: {byte_len} bytes is not a multiple of {bytes_per_record} ({trailing_bytes} trailing bytes)",
                path.display()
            ),
            Self::RecordIndexOutOfRange {
                index,
                record_count,
            } => write!(
                f,
                "record {index} is out of range: the corpus holds {record_count} records"
            ),
            Self::EmptySourceList => {
                write!(f, "no source corpus files were given to read")
            }
            Self::RecordLengthMismatch {
                path,
                bytes_per_record,
                actual,
            } => write!(
                f,
                "derived corpus {}: a record is {bytes_per_record} bytes, but {actual} bytes were offered",
                path.display()
            ),
            Self::UnsupportedSourceKind { path } => write!(
                f,
                "source {} is neither a regular file nor a directory",
                path.display()
            ),
            Self::NoSources { path } => write!(
                f,
                "source directory {} contains no corpus files",
                path.display()
            ),
            Self::DestinationIsSource { path } => write!(
                f,
                "derived destination {} is a source corpus — sources are immutable",
                path.display()
            ),
            Self::InvalidDestination { path } => write!(
                f,
                "derived destination {} has no file name",
                path.display()
            ),
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
        }
    }
}

impl Error for CorpusError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
