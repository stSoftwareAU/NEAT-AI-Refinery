//! Re-verifying a published corpus from the outside.
//!
//! The sampler already validates what it writes. A soak re-opens the published
//! directory afterwards and checks it against its own `manifest.json` — the
//! artefact a consumer actually reads — so the evidence is what a downstream
//! caller would see, not what the producing run believed it wrote.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::SoakError;
use crate::corpus::RecordShape;
use crate::manifest::{Checksum, Manifest, RecordGeometry, MANIFEST_FILE_NAME};

/// A published derived corpus, verified against its own provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedCorpus {
    /// The published directory.
    pub directory: PathBuf,
    /// The corpus file inside it.
    pub file: String,
    /// Its byte length.
    pub bytes: u64,
    /// Records it holds, derived from the bytes rather than believed.
    pub record_count: u64,
    /// Records the run read to produce it.
    pub records_read: u64,
    /// The SHA-256 of the published bytes.
    pub checksum: String,
}

impl PublishedCorpus {
    /// Verifies the corpus published at `directory` against `shape`.
    ///
    /// Checked, in order: the manifest parses; its geometry is the shape that
    /// was asked for; the named corpus file is the byte length recorded; those
    /// bytes divide into whole records; the record count matches the manifest;
    /// the bytes still hash to the published checksum; and the sample is no
    /// larger than the corpus it was drawn from.
    ///
    /// # Errors
    ///
    /// Returns [`SoakError::Manifest`] when the manifest cannot be read,
    /// [`SoakError::Io`] when the corpus file cannot be measured, and
    /// [`SoakError::Invariant`] naming the first check that did not hold.
    pub fn verify(directory: impl AsRef<Path>, shape: RecordShape) -> Result<Self, SoakError> {
        let directory = directory.as_ref();
        let manifest = Manifest::load(directory.join(MANIFEST_FILE_NAME))?;

        let expected: RecordGeometry = shape.into();
        if manifest.record_shape != expected {
            return Err(SoakError::invariant(
                "record geometry",
                format!(
                    "{} was published at {:?}, not the requested {:?}",
                    directory.display(),
                    manifest.record_shape,
                    expected
                ),
            ));
        }

        let corpus_file = directory.join(&manifest.output.file);
        let bytes = std::fs::metadata(&corpus_file)
            .map_err(|e| SoakError::io(&corpus_file, e))?
            .len();
        if bytes != manifest.output.bytes {
            return Err(SoakError::invariant(
                "published bytes",
                format!(
                    "{} holds {bytes} bytes, the manifest records {}",
                    corpus_file.display(),
                    manifest.output.bytes
                ),
            ));
        }

        let bytes_per_record = manifest.record_shape.bytes_per_record as u64;
        let remainder = bytes % bytes_per_record;
        if remainder != 0 {
            return Err(SoakError::invariant(
                "whole records",
                format!(
                    "{} holds {bytes} bytes — {remainder} beyond a whole {bytes_per_record}-byte record",
                    corpus_file.display()
                ),
            ));
        }

        let record_count = bytes / bytes_per_record;
        if record_count != manifest.output.record_count {
            return Err(SoakError::invariant(
                "record count",
                format!(
                    "{} holds {record_count} records, the manifest records {}",
                    corpus_file.display(),
                    manifest.output.record_count
                ),
            ));
        }

        let checksum = Checksum::of_file(&corpus_file)?;
        if checksum.value != manifest.output.checksum.value {
            return Err(SoakError::invariant(
                "published checksum",
                format!(
                    "{} hashes to {}, the manifest records {}",
                    corpus_file.display(),
                    checksum.value,
                    manifest.output.checksum.value
                ),
            ));
        }

        if record_count > manifest.source.record_count {
            return Err(SoakError::invariant(
                "sample size",
                format!(
                    "{} kept {record_count} of {} records read",
                    corpus_file.display(),
                    manifest.source.record_count
                ),
            ));
        }

        Ok(Self {
            directory: directory.to_path_buf(),
            file: manifest.output.file,
            bytes,
            record_count,
            records_read: manifest.source.record_count,
            checksum: checksum.value,
        })
    }
}
