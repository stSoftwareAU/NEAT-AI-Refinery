//! Source discovery and its ordering rules.
//!
//! Discovery is deliberately dull and deterministic — a derived corpus is only
//! reproducible if the same source path always yields the same list, in the
//! same order, on every machine.

use std::fs;
use std::path::{Path, PathBuf};

use super::CorpusError;

/// Expands a source path into the corpus files to read, in read order.
///
/// The rules are:
///
/// 1. A regular file is used as-is and yields exactly that one path.
/// 2. A directory is scanned **non-recursively**: nested directories are
///    skipped, never descended into.
/// 3. Entries whose name begins with `.` are skipped, so editor swap files and
///    other dot-files never join a corpus.
/// 4. Remaining entries must resolve to regular files (a symlink to a regular
///    file qualifies); anything else in the directory is skipped.
/// 5. The result is sorted by file name, byte-wise — not by locale, case or
///    embedded number. `Shard-1` precedes `shard-10`, which precedes `shard-2`.
///
/// Discovery only reads directory metadata; no source is opened, modified or
/// created here.
///
/// # Errors
///
/// Returns [`CorpusError::NoSources`] for a directory holding no corpus files,
/// [`CorpusError::UnsupportedSourceKind`] for a path that is neither a regular
/// file nor a directory, and [`CorpusError::Io`] when the path cannot be read.
pub fn discover_sources(path: impl AsRef<Path>) -> Result<Vec<PathBuf>, CorpusError> {
    let path = path.as_ref();
    let metadata = fs::metadata(path).map_err(|e| CorpusError::io(path, e))?;

    if metadata.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    if !metadata.is_dir() {
        return Err(CorpusError::UnsupportedSourceKind {
            path: path.to_path_buf(),
        });
    }

    let mut sources = Vec::new();
    for entry in fs::read_dir(path).map_err(|e| CorpusError::io(path, e))? {
        let entry = entry.map_err(|e| CorpusError::io(path, e))?;
        let name = entry.file_name();
        if name.as_encoded_bytes().first() == Some(&b'.') {
            continue;
        }
        let candidate = entry.path();
        // `fs::metadata` follows symlinks, so a link to a corpus file counts.
        let Ok(candidate_meta) = fs::metadata(&candidate) else {
            continue;
        };
        if candidate_meta.is_file() {
            sources.push(candidate);
        }
    }

    if sources.is_empty() {
        return Err(CorpusError::NoSources {
            path: path.to_path_buf(),
        });
    }

    sources.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    Ok(sources)
}
