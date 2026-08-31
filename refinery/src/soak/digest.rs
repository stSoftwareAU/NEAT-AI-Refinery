//! Fingerprinting a corpus directory, so a run that changed it cannot pass.
//!
//! The immutable-source rule is the one property a soak cannot take on trust:
//! it is the difference between a derived corpus and a damaged training set.
//! A soak therefore digests the source before the first run and after the
//! last, and compares. Unlike a manifest's `path+bytes` identity — which is
//! about naming a source cheaply — this hashes the content, because equal
//! byte lengths would hide an in-place edit.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::SoakError;
use crate::corpus::discover_sources;
use crate::manifest::Checksum;

/// One file inside a digested corpus directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDigest {
    /// The file name inside the directory.
    pub name: String,
    /// Its byte length.
    pub bytes: u64,
    /// The SHA-256 of its contents, lower-case hexadecimal.
    pub sha256: String,
}

/// The content fingerprint of a corpus directory, in discovery order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusDigest {
    /// Every file discovered, in the order `discover_sources` returns them.
    pub files: Vec<FileDigest>,
}

impl CorpusDigest {
    /// Digests every file in `directory`.
    ///
    /// Discovery is the same non-recursive, dot-file-skipping, byte-wise
    /// sorted walk the sampler itself uses, so a digest covers exactly the
    /// files a run would have read — and, applied to a published corpus,
    /// covers its manifest as well as its records.
    ///
    /// # Errors
    ///
    /// Returns [`SoakError::Corpus`] when the directory cannot be walked and
    /// [`SoakError::Manifest`] when a file cannot be read or hashed.
    pub fn of(directory: impl AsRef<Path>) -> Result<Self, SoakError> {
        let mut files = Vec::new();
        for path in discover_sources(directory.as_ref())? {
            let bytes = std::fs::metadata(&path)
                .map_err(|e| SoakError::io(&path, e))?
                .len();
            files.push(FileDigest {
                name: path
                    .file_name()
                    .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
                bytes,
                sha256: Checksum::of_file(&path)?.value,
            });
        }
        Ok(Self { files })
    }
}
