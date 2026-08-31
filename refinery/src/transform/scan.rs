//! Finding the corpus files a transform reads, and proving the destination is
//! not one of them.

use std::fs;
use std::path::{Path, PathBuf};

use super::TransformError;
use crate::corpus::discover_sources;
use crate::manifest::{Manifest, SourceFile, MANIFEST_FILE_NAME};

/// The `.bin` corpus files in `source`, in discovery order.
///
/// Discovery is non-recursive and skips dot-files; narrowing it to `.bin`
/// files means a stray note, checksum or `manifest.json` beside the corpus is
/// not mistaken for records — which is what lets one transform read the
/// published output of another.
///
/// # Errors
///
/// Returns [`TransformError::NoCorpusFiles`] when the directory holds none,
/// and [`TransformError::Corpus`] when it cannot be scanned.
pub fn corpus_files(source: &Path) -> Result<Vec<PathBuf>, TransformError> {
    let files: Vec<PathBuf> = discover_sources(source)?
        .into_iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "bin"))
        .collect();

    if files.is_empty() {
        return Err(TransformError::NoCorpusFiles {
            path: source.to_path_buf(),
        });
    }
    Ok(files)
}

/// The manifest a Refinery-published source carries, when it has one.
///
/// A transform reading another transform's output need not take the caller's
/// word for the corpus layout: the manifest states it. A raw training corpus
/// has none, and `Ok(None)` says so. A manifest that is present but unreadable
/// is a fault rather than an absence — reading past it would be guessing at the
/// corpus it describes.
///
/// # Errors
///
/// Returns [`TransformError::Manifest`] when a manifest is present and cannot
/// be read.
pub fn source_manifest(source: &Path) -> Result<Option<Manifest>, TransformError> {
    let path = source.join(MANIFEST_FILE_NAME);
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(Manifest::load(&path)?))
}

/// The path a source corpus' manifest would occupy, for an error to name.
#[must_use]
pub fn source_manifest_path(source: &Path) -> PathBuf {
    source.join(MANIFEST_FILE_NAME)
}

/// Identifies one source file by name and byte length.
///
/// # Errors
///
/// Returns [`TransformError::Io`] when the file cannot be inspected.
pub fn source_file(path: &Path) -> Result<SourceFile, TransformError> {
    Ok(SourceFile {
        name: path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
        bytes: file_bytes(path)?,
    })
}

/// The byte length of `path`.
///
/// # Errors
///
/// Returns [`TransformError::Io`] when the file cannot be inspected.
pub fn file_bytes(path: &Path) -> Result<u64, TransformError> {
    Ok(fs::metadata(path)
        .map_err(|e| TransformError::io(path, e))?
        .len())
}

/// Rejects an output directory that overlaps the source corpus, returning the
/// canonical source path the manifest records.
///
/// Publishing renames the whole output directory aside and deletes it, so
/// either nesting is fatal: an output inside the source, and a source inside
/// the output, both put an immutable source corpus one rename away from
/// deletion. Resolving both paths first means a relative path, a `..` segment
/// or a symlink cannot hide the overlap.
///
/// # Errors
///
/// Returns [`TransformError::OverlappingCorpora`] when they overlap, and
/// [`TransformError::Io`] when either path cannot be resolved.
pub fn resolved_source(source: &Path, output: &Path) -> Result<PathBuf, TransformError> {
    let resolved_source = fs::canonicalize(source).map_err(|e| TransformError::io(source, e))?;
    let resolved_output = resolve_output(output)?;

    if resolved_output.starts_with(&resolved_source)
        || resolved_source.starts_with(&resolved_output)
    {
        return Err(TransformError::OverlappingCorpora {
            output: resolved_output,
            source: resolved_source,
        });
    }
    Ok(resolved_source)
}

/// Resolves the output directory, which need not exist yet.
///
/// An existing path is canonicalised; otherwise its parent is, and the name is
/// re-joined. The parent must already exist — a derived corpus under a missing
/// directory is a caller mistake worth failing on before any file is read.
fn resolve_output(output: &Path) -> Result<PathBuf, TransformError> {
    if output.exists() {
        return fs::canonicalize(output).map_err(|e| TransformError::io(output, e));
    }

    let name = output.file_name().ok_or_else(|| {
        TransformError::io(
            output,
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "a derived corpus directory needs a file name",
            ),
        )
    })?;
    let parent = match output.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let parent = fs::canonicalize(&parent).map_err(|e| TransformError::io(&parent, e))?;

    Ok(parent.join(name))
}
