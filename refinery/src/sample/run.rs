//! The sampling run itself.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{RngExt, SeedableRng};

use super::{SampleError, SampleRate, SampleRequest};
use crate::corpus::{DerivedDestination, RecordReader, RecordWriter};
use crate::manifest::{Checksum, Manifest, OutputArtefact, SourceIdentity, TransformRecord};
use crate::transform::{corpus_files, file_bytes, resolved_source, source_file, StagedCorpus};

/// The transform name recorded in the manifest.
const TRANSFORM_NAME: &str = "sample";

/// The divisor of the headroom added to an estimated pass: a hundredth, so a
/// reported requirement covers the manifest and a sample above its mean.
const HEADROOM_DIVISOR: u64 = 100;

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
    let sources = corpus_files(source)?;

    // Measured before a byte is written: a run that stops for want of space
    // has to say what a whole fresh attempt costs, and the figure is only
    // cheap to obtain while the sources are still in hand.
    let whole_pass = pass_bytes(&sources, request)?;
    let mut written = 0_u64;

    run(request, resolved_source, sources, &mut written)
        // A retry writes the whole pass again, so the requirement is that
        // pass — never less than this attempt had already put on the volume.
        .map_err(|error| error.with_required_bytes(whole_pass.max(written)))
}

/// The bytes a whole sampling pass publishes, worked out before it is run.
///
/// This is the figure a run that fails for want of space reports as its
/// requirement: a fresh attempt re-reads every source file and writes the
/// derived corpus from scratch, so the space it needs is the size of a whole
/// pass rather than the remainder of the one that failed.
///
/// # Errors
///
/// Returns [`SampleError::NoCorpusFiles`] for a source directory with no
/// `.bin` files, and [`SampleError::Io`] when a source file cannot be
/// inspected.
pub fn whole_pass_bytes(request: &SampleRequest) -> Result<u64, SampleError> {
    let sources = corpus_files(&request.source)?;
    pass_bytes(&sources, request)
}

/// The size of a whole pass over `sources`, headroom included.
fn pass_bytes(sources: &[PathBuf], request: &SampleRequest) -> Result<u64, SampleError> {
    let mut source_bytes = 0_u64;
    for path in sources {
        source_bytes = source_bytes.saturating_add(file_bytes(path)?);
    }
    Ok(expected_bytes(
        source_bytes,
        request.shape.bytes_per_record(),
        request.rate,
    ))
}

/// The bytes a pass over `source_bytes` of fixed-width records publishes.
///
/// Records are never split, so a pass keeping each of them with probability
/// `rate` writes `ceil(records × rate)` whole records. One per cent is added
/// on top, in integer maths, for the manifest published beside the corpus and
/// for a sample that lands above its mean.
fn expected_bytes(source_bytes: u64, bytes_per_record: usize, rate: SampleRate) -> u64 {
    // A record shape always holds at least one value, so the width is never
    // zero and the division is always defined.
    let width = bytes_per_record as u64;
    let records = source_bytes / width;
    let kept = kept_records(records, rate.value());
    let corpus = kept.saturating_mul(width);

    corpus.saturating_add(corpus.div_ceil(HEADROOM_DIVISOR))
}

/// Records a pass keeps out of `records`, rounded up: the sample is a random
/// variable, and reporting less than the mean would under-state the space.
fn kept_records(records: u64, rate: f64) -> u64 {
    let kept = (records as f64 * rate).ceil();
    if kept.is_finite() && kept > 0.0 {
        // `rate` is at most 1, so this never exceeds the records read.
        (kept as u64).min(records)
    } else {
        0
    }
}

/// Runs the pass itself, reporting the bytes it wrote through `written`.
fn run(
    request: &SampleRequest,
    resolved_source: PathBuf,
    mut sources: Vec<PathBuf>,
    written: &mut u64,
) -> Result<SampleOutcome, SampleError> {
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
        records_read += sample_file(path, request, &mut rng, &mut writer, written)?;
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
    written: &mut u64,
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
        // Tracked as it goes, so a run that stops for want of space knows what
        // it had already put on the volume.
        *written = written.saturating_add(record.len() as u64);
    }

    Ok(records_read)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::corpus::RecordShape;
    use crate::manifest::CallerMetadata;

    /// Bytes one record of the test shape occupies: three `f32` values.
    const RECORD_BYTES: u64 = 12;

    /// A throwaway source directory holding two corpus files, of
    /// `first_records` and `second_records` records.
    fn two_file_source(label: &str, first_records: u64, second_records: u64) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "refinery-pass-bytes-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("create the source directory");

        for (name, records) in [
            ("shard-1.bin", first_records),
            ("shard-2.bin", second_records),
        ] {
            let bytes = vec![0_u8; usize::try_from(records * RECORD_BYTES).expect("a small file")];
            fs::write(directory.join(name), &bytes).expect("write a corpus file");
        }
        directory
    }

    /// A sampling request over `source` at `rate`.
    fn request(source: &Path, rate: f64) -> SampleRequest {
        SampleRequest {
            source: source.to_path_buf(),
            output: source.with_extension("derived"),
            shape: RecordShape::new(2, 1).expect("a three-value record shape"),
            rate: SampleRate::new(rate).expect("a valid rate"),
            seed: Some(7),
            metadata: CallerMetadata::default(),
        }
    }

    #[test]
    fn a_whole_pass_at_full_rate_is_the_source_plus_headroom() {
        let source = two_file_source("full-rate", 40, 60);
        let sum = (40 + 60) * RECORD_BYTES;

        let estimate = whole_pass_bytes(&request(&source, 1.0)).expect("estimate the pass");

        assert_eq!(
            estimate,
            sum + sum / 100,
            "a rate of 1 keeps every record, so the pass is the whole source plus 1%"
        );
        let _ = fs::remove_dir_all(&source);
    }

    #[test]
    fn a_whole_pass_at_a_sampling_rate_is_that_share_plus_headroom() {
        let source = two_file_source("sampling-rate", 40, 60);
        let sum = (40 + 60) * RECORD_BYTES;
        let share = 60_u64; // ceil(0.05 × 1200) — five of the hundred records.

        let estimate = whole_pass_bytes(&request(&source, 0.05)).expect("estimate the pass");

        assert_eq!(estimate, share + share.div_ceil(100));
        assert!(
            estimate >= (sum as f64 * 0.05).ceil() as u64
                && estimate <= (sum as f64 * 0.05 * 1.02).ceil() as u64,
            "the estimate covers a whole pass without over-stating it: {estimate}"
        );
        let _ = fs::remove_dir_all(&source);
    }

    #[test]
    fn a_partial_record_at_the_end_of_a_source_is_not_counted() {
        // Records are never split, so a trailing partial one buys no space in
        // the derived corpus — and no reader would accept it either.
        let whole = expected_bytes(
            10 * RECORD_BYTES,
            RECORD_BYTES as usize,
            SampleRate::new(1.0).expect("a valid rate"),
        );
        let ragged = expected_bytes(
            10 * RECORD_BYTES + 5,
            RECORD_BYTES as usize,
            SampleRate::new(1.0).expect("a valid rate"),
        );

        assert_eq!(whole, ragged);
    }

    #[test]
    fn an_empty_source_needs_no_space() {
        assert_eq!(
            expected_bytes(
                0,
                RECORD_BYTES as usize,
                SampleRate::new(0.5).expect("a valid rate")
            ),
            0
        );
    }

    #[test]
    fn a_rate_below_one_record_still_reserves_a_whole_one() {
        // Ten records at a rate of 0.01 average a tenth of a record; a run that
        // keeps one writes a whole one, so that is what a retry must fit.
        let estimate = expected_bytes(
            10 * RECORD_BYTES,
            RECORD_BYTES as usize,
            SampleRate::new(0.01).expect("a valid rate"),
        );

        assert_eq!(estimate, RECORD_BYTES + 1);
    }

    #[test]
    fn a_source_that_cannot_be_scanned_is_reported_rather_than_guessed() {
        let missing = std::env::temp_dir().join(format!(
            "refinery-pass-bytes-missing-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&missing);

        let error = whole_pass_bytes(&request(&missing, 0.05))
            .expect_err("a source that is not there cannot be estimated");

        assert!(
            matches!(
                error,
                SampleError::NoCorpusFiles { .. } | SampleError::Corpus(_) | SampleError::Io { .. }
            ),
            "{error:?}"
        );
    }
}
