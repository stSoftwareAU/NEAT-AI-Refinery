//! The quantisation run itself.

use std::path::{Path, PathBuf};

use super::{QuantiseError, QuantiseRequest};
use crate::corpus::{DerivedDestination, RecordReader, RecordWriter};
use crate::manifest::{
    Checksum, Manifest, OutputArtefact, SourceIdentity, TransformRecord, MANIFEST_FILE_NAME,
};
use crate::transform::{corpus_files, file_bytes, resolved_source, source_file, StagedCorpus};

/// The transform name recorded in the manifest.
const TRANSFORM_NAME: &str = "quantise";

/// What a completed quantisation run produced.
#[derive(Debug, Clone)]
pub struct QuantiseOutcome {
    /// The corpus files read, in discovery order.
    pub sources: Vec<PathBuf>,
    /// Records read across every source.
    pub records_read: u64,
    /// Records written — always equal to [`QuantiseOutcome::records_read`],
    /// because quantisation re-encodes records rather than selecting them.
    pub records_written: u64,
    /// Bytes the source corpus occupied.
    pub source_bytes: u64,
    /// Bytes the published corpus occupies.
    pub output_bytes: u64,
    /// The published corpus file.
    pub output_file: PathBuf,
    /// The published manifest file.
    pub manifest_file: PathBuf,
    /// The provenance record published beside the corpus.
    pub manifest: Manifest,
}

impl QuantiseOutcome {
    /// The share of the source bytes the published corpus saved, in `[0, 1]`.
    ///
    /// Zero when the source was empty, so a caller never divides by zero to
    /// report a reduction that was never measured.
    #[must_use]
    pub fn storage_reduction(&self) -> f64 {
        if self.source_bytes == 0 {
            return 0.0;
        }
        1.0 - (self.output_bytes as f64 / self.source_bytes as f64)
    }
}

/// Re-encodes the source corpus under the requested scheme and publishes the
/// result as a fresh derived corpus.
///
/// The source is only ever read. Records keep their order and their count —
/// only their representation changes — and the derived corpus is built in a
/// staging directory, manifest included, and published with an atomic rename.
///
/// # Errors
///
/// Returns [`QuantiseError::SourceEncodingMismatch`] or
/// [`QuantiseError::SourceWidthMismatch`] when the source's own manifest
/// contradicts what the run was asked to read, and
/// [`QuantiseError::Transform`] for a missing corpus, an overlapping
/// destination, a malformed record, a failed write, provenance that cannot be
/// produced, or a failed publish.
pub fn quantise(request: &QuantiseRequest) -> Result<QuantiseOutcome, QuantiseError> {
    let source = &request.source;
    let resolved = resolved_source(source, &request.output)?;

    let source_shape = request.shape;
    let target_shape = request.target_shape()?;
    check_source_declaration(source, request)?;

    let sources = corpus_files(source)?;
    let mut read_files = Vec::with_capacity(sources.len());
    let mut source_bytes = 0_u64;
    for path in &sources {
        let file = source_file(path)?;
        source_bytes += file.bytes;
        read_files.push(file);
    }

    let staged = StagedCorpus::create(&request.output)?;
    let file_name = request.scheme.file_name();
    let destination = DerivedDestination::new(staged.path().join(&file_name), &sources)?;
    let mut writer = RecordWriter::create(&destination, target_shape)?;

    // One scratch record, reused: the working set stays the reader's buffer
    // plus a single record however large the corpus is.
    let mut encoded = Vec::with_capacity(target_shape.bytes_per_record());
    let mut reader = RecordReader::open(&sources, source_shape)?;
    while let Some(record) = reader.next_record() {
        let record = record?;
        encoded.clear();
        source_shape
            .encoding()
            .transcode_into(record, target_shape.encoding(), &mut encoded);
        writer.write_record(&encoded)?;
    }
    let records_read = reader.records_read();
    let records_written = writer.finish()?;

    let staged_file = staged.path().join(&file_name);
    let output_bytes = file_bytes(&staged_file)?;
    let manifest = Manifest::new(
        TransformRecord::new(TRANSFORM_NAME, request.scheme.parameters(), None),
        target_shape.into(),
        SourceIdentity::new(resolved, read_files, records_read),
        OutputArtefact {
            file: file_name.clone(),
            record_count: records_written,
            bytes: output_bytes,
            checksum: Checksum::of_file(&staged_file)?,
        },
        request.metadata.clone(),
    )
    // A quantised corpus is not stored the way its source was, so the source
    // layout is recorded explicitly rather than inferred from the output.
    .with_source_record_shape(source_shape.into());
    manifest.write_into(staged.path())?;

    let output_file = staged.destination().join(&file_name);
    let manifest_file = staged.destination().join(MANIFEST_FILE_NAME);
    staged.publish()?;

    Ok(QuantiseOutcome {
        sources,
        records_read,
        records_written,
        source_bytes,
        output_bytes,
        output_file,
        manifest_file,
        manifest,
    })
}

/// Checks the source's own manifest, when it has one, against what this run
/// was told to read.
///
/// A Refinery-published corpus carries its layout beside it, so composing
/// transforms need not take the caller's word for it. Quantising an already
/// quantised corpus, or reading one at the wrong record width, is caught here
/// rather than producing a corpus of reinterpreted bytes. A source with no
/// manifest — a raw training corpus — is read as the caller described it.
fn check_source_declaration(source: &Path, request: &QuantiseRequest) -> Result<(), QuantiseError> {
    let path = source.join(MANIFEST_FILE_NAME);
    if !path.exists() {
        return Ok(());
    }

    // A manifest that is present but unreadable is a fault, not an absence:
    // reading past it would be guessing at the corpus it describes.
    let manifest = Manifest::load(&path)?;
    let expected = request.scheme.source_encoding();

    if manifest.record_shape.encoding != expected.name() {
        return Err(QuantiseError::SourceEncodingMismatch {
            manifest: path,
            expected: expected.name().to_string(),
            found: manifest.record_shape.encoding,
        });
    }
    if manifest.record_shape.bytes_per_record != request.shape.bytes_per_record() {
        return Err(QuantiseError::SourceWidthMismatch {
            manifest: path,
            expected: request.shape.bytes_per_record(),
            found: manifest.record_shape.bytes_per_record,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(source_bytes: u64, output_bytes: u64) -> QuantiseOutcome {
        QuantiseOutcome {
            sources: Vec::new(),
            records_read: 0,
            records_written: 0,
            source_bytes,
            output_bytes,
            output_file: PathBuf::new(),
            manifest_file: PathBuf::new(),
            manifest: Manifest::new(
                TransformRecord::new(TRANSFORM_NAME, Default::default(), None),
                crate::corpus::RecordShape::new(1, 1)
                    .expect("valid shape")
                    .into(),
                SourceIdentity::new(PathBuf::new(), Vec::new(), 0),
                OutputArtefact {
                    file: String::new(),
                    record_count: 0,
                    bytes: 0,
                    checksum: Checksum {
                        algorithm: "sha256".to_string(),
                        value: "00".repeat(32),
                    },
                },
                Default::default(),
            ),
        }
    }

    #[test]
    fn reports_the_share_of_bytes_a_run_saved() {
        assert!((outcome(1000, 500).storage_reduction() - 0.5).abs() < f64::EPSILON);
        assert!((outcome(1000, 1000).storage_reduction()).abs() < f64::EPSILON);
    }

    #[test]
    fn reports_no_reduction_rather_than_dividing_by_zero() {
        assert_eq!(outcome(0, 0).storage_reduction(), 0.0);
    }
}
