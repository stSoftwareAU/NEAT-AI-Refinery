//! The sampling run itself.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{RngExt, SeedableRng};

use super::{SampleError, SampleRequest};
use crate::corpus::{DerivedDestination, RecordReader, RecordWriter};
use crate::manifest::{Checksum, Manifest, OutputArtefact, SourceIdentity, TransformRecord};
use crate::transform::{corpus_files, file_bytes, resolved_source, source_file, StagedCorpus};

/// The transform name recorded in the manifest.
const TRANSFORM_NAME: &str = "sample";

/// What a completed sampling run produced.
#[derive(Debug, Clone)]
pub struct SampleOutcome {
    /// The corpus files read, in the randomised order they were processed.
    pub sources: Vec<PathBuf>,
    /// Records read across every source.
    pub records_read: u64,
    /// Records kept and published.
    pub records_written: u64,
    /// The published corpus file.
    pub output_file: PathBuf,
    /// The seed the run used — supplied, or drawn from the operating system.
    pub seed: u64,
    /// The published manifest file.
    pub manifest_file: PathBuf,
    /// The provenance record published beside the corpus.
    pub manifest: Manifest,
}

/// Samples the source corpus into a freshly published derived corpus.
///
/// The source is only ever read. The derived corpus is built in a staging
/// directory — corpus and manifest together — and published with an atomic
/// rename, so the live directory is replaced whole or not at all, and a
/// published corpus always carries its provenance.
///
/// # Errors
///
/// Returns [`SampleError::NoCorpusFiles`] for a source directory with no
/// `.bin` files, [`SampleError::OverlappingCorpora`] when the derived corpus
/// and the source overlap on disk, [`SampleError::Corpus`] for a malformed
/// record or a failed write, [`SampleError::Manifest`] when the provenance
/// record cannot be produced, [`SampleError::Publish`] when the swap fails,
/// and [`SampleError::Io`] for any other filesystem failure.
pub fn sample(request: &SampleRequest) -> Result<SampleOutcome, SampleError> {
    let source = &request.source;
    let resolved_source = resolved_source(source, &request.output)?;

    let mut sources = corpus_files(source)?;
    let seed = request.seed.unwrap_or_else(|| rand::rng().random());
    let mut rng = StdRng::seed_from_u64(seed);

    // Input files are processed in random order, as the Deno sampler does.
    sources.shuffle(&mut rng);

    let staged = StagedCorpus::create(&request.output)?;
    let file_name = request.rate.file_name();
    let destination = DerivedDestination::new(staged.path().join(&file_name), &sources)?;
    let mut writer = RecordWriter::create(&destination, request.shape)?;

    let mut records_read = 0_u64;
    let mut read_files = Vec::with_capacity(sources.len());
    for path in &sources {
        read_files.push(source_file(path)?);
        records_read += sample_file(path, request, &mut rng, &mut writer)?;
    }
    let records_written = writer.finish()?;

    // Provenance is written into the staging directory, so the publishing
    // rename carries the corpus and its manifest across together. A manifest
    // that cannot be written aborts the run with nothing published.
    let staged_file = staged.path().join(&file_name);
    let manifest = Manifest::new(
        TransformRecord::new(TRANSFORM_NAME, parameters(request), Some(seed)),
        request.shape.into(),
        SourceIdentity::new(resolved_source, read_files, records_read),
        OutputArtefact {
            file: file_name.clone(),
            record_count: records_written,
            bytes: file_bytes(&staged_file)?,
            checksum: Checksum::of_file(&staged_file)?,
        },
        request.metadata.clone(),
    );
    manifest.write_into(staged.path())?;

    let output_file = staged.destination().join(&file_name);
    let manifest_file = staged
        .destination()
        .join(crate::manifest::MANIFEST_FILE_NAME);
    staged.publish()?;

    Ok(SampleOutcome {
        sources,
        records_read,
        records_written,
        output_file,
        seed,
        manifest_file,
        manifest,
    })
}

/// The transform parameters as the manifest records them.
///
/// Only what the caller can vary is recorded — the output file name follows
/// from the rate, so it lives in the output section rather than here.
fn parameters(request: &SampleRequest) -> BTreeMap<String, serde_json::Value> {
    let mut parameters = BTreeMap::new();
    parameters.insert(
        "rate".to_string(),
        serde_json::Value::from(request.rate.value()),
    );
    parameters
}

/// Streams one corpus file, keeping each record with probability `rate`, and
/// appends the kept records in a random order.
///
/// The kept records — not the file — are what is held in memory, exactly as in
/// the Deno sampler: the working set is one file's share of the sample.
fn sample_file(
    path: &Path,
    request: &SampleRequest,
    rng: &mut StdRng,
    writer: &mut RecordWriter,
) -> Result<u64, SampleError> {
    let rate = request.rate.value();
    let only = [path.to_path_buf()];
    let mut reader = RecordReader::open(&only, request.shape)?;

    let mut kept: Vec<Vec<u8>> = Vec::new();
    while let Some(record) = reader.next_record() {
        let record = record?;
        // Each record is an independent Bernoulli trial: `random` yields
        // `[0, 1)`, so a rate of 1 keeps everything.
        if rng.random::<f64>() < rate {
            kept.push(record.to_vec());
        }
    }
    let records_read = reader.records_read();

    kept.shuffle(rng);
    for record in &kept {
        writer.write_record(record)?;
    }

    Ok(records_read)
}
