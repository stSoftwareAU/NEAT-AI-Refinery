//! Separation between a derived corpus and the sources it was derived from.

use std::fs;
use std::path::{Path, PathBuf};

use super::CorpusError;

/// A checked destination for a derived corpus.
///
/// Constructing one proves the destination is not a source: the derived path
/// and every source are resolved against the filesystem first, so a relative
/// path, a `..` segment or a symlinked directory cannot smuggle a write back
/// onto a source corpus.
///
/// Holding a `DerivedDestination` grants no write capability by itself — this
/// crate never writes. It is the checked path a writer is handed later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedDestination {
    path: PathBuf,
}

impl DerivedDestination {
    /// Checks `path` as the destination for a corpus derived from `sources`.
    ///
    /// # Errors
    ///
    /// Returns [`CorpusError::DestinationIsSource`] when the destination
    /// resolves to one of `sources`, [`CorpusError::InvalidDestination`] when
    /// it has no file name, and [`CorpusError::Io`] when a path cannot be
    /// resolved — an absent parent directory included.
    pub fn new(path: impl AsRef<Path>, sources: &[PathBuf]) -> Result<Self, CorpusError> {
        let path = path.as_ref();
        let resolved = resolve(path)?;

        for source in sources {
            if resolve(source)? == resolved {
                return Err(CorpusError::DestinationIsSource {
                    path: path.to_path_buf(),
                });
            }
        }

        Ok(Self { path: resolved })
    }

    /// The resolved destination path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Resolves `path` to an absolute, symlink-free path.
///
/// An existing path is canonicalised directly. A path that does not exist yet
/// — the usual case for a derived corpus — has its parent directory
/// canonicalised instead, and the file name is re-joined; the parent must
/// already exist, because a destination under a missing directory is a caller
/// mistake worth failing on now rather than at write time.
fn resolve(path: &Path) -> Result<PathBuf, CorpusError> {
    if path.exists() {
        return fs::canonicalize(path).map_err(|e| CorpusError::io(path, e));
    }

    let name = path
        .file_name()
        .ok_or_else(|| CorpusError::InvalidDestination {
            path: path.to_path_buf(),
        })?;
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let parent = fs::canonicalize(parent).map_err(|e| CorpusError::io(parent, e))?;

    Ok(parent.join(name))
}
