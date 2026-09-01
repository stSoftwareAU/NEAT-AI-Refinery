//! The sampling run itself.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{RngExt, SeedableRng};

use super::{SampleError, SampleRequest};
use crate::corpus::{DerivedDestination, RecordReader, RecordWriter};
use crate::exit::is_storage_full;
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

    // What this pass expects to write, measured before a byte of it is
    // written, so a run stopped by a full volume can say what it still needs.
    let plan = OutputPlan::new(&sources, request)?;

    let staged = StagedCorpus::create(&request.output)?;
    let file_name = request.rate.file_name();
    let destination = DerivedDestination::new(staged.path().join(&file_name), &sources)?;
    let mut writer = RecordWriter::create(&destination, request.shape)?;

    let mut records_read = 0_u64;
    let mut read_files = Vec::with_capacity(sources.len());
    for path in &sources {
        read_files.push(source_file(path)?);
        match sample_file(path, request, &mut rng, &mut writer) {
            Ok(count) => records_read += count,
            Err(error) => return Err(plan.explain(error, writer.records_written())),
        }
    }
    // Taken before `finish` consumes the writer: a tail flush is the write most
    // likely to be the one the volume has no room for.
    let buffered = writer.records_written();
    let records_written = match writer.finish() {
        Ok(count) => count,
        Err(error) => return Err(plan.explain(error.into(), buffered)),
    };

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

/// What a sampling pass expects to write, so a run stopped by a full volume
/// can report the space it still needs rather than only that it failed.
///
/// The estimate is the corpus arithmetic itself: every source record is an
/// independent Bernoulli trial at `rate`, so the pass expects `rate` of the
/// source records, each exactly one record wide.
#[derive(Debug, Clone, Copy)]
struct OutputPlan {
    /// Records the whole pass expects to write.
    expected_records: u64,
    /// Bytes each of them occupies.
    bytes_per_record: u64,
}

impl OutputPlan {
    /// Plans the output of `request` over the corpus files it will read.
    ///
    /// # Errors
    ///
    /// Returns [`SampleError::Io`] when a source file cannot be measured.
    fn new(sources: &[PathBuf], request: &SampleRequest) -> Result<Self, SampleError> {
        // A shape is validated non-zero, so this divisor never is.
        let bytes_per_record = request.shape.bytes_per_record() as u64;
        let mut source_records = 0_u64;
        for path in sources {
            source_records = source_records.saturating_add(file_bytes(path)? / bytes_per_record);
        }

        let expected = (source_records as f64 * request.rate.value()).round();
        Ok(Self {
            expected_records: expected as u64,
            bytes_per_record,
        })
    }

    /// Restates an out-of-space failure with the space another attempt needs.
    ///
    /// Any other failure is returned untouched — the figures answer one
    /// question, "is another attempt worth spending?", and only a full volume
    /// asks it.
    ///
    /// The figure is the **whole** planned corpus, not the part still
    /// unwritten. Nothing resumes a half-written corpus: the next attempt
    /// re-reads every source from the first record, and the caller sweeping
    /// scratch between attempts deletes the partial output before it starts. A
    /// remainder would therefore approve a retry the volume cannot hold — which
    /// is the failure this reporting exists to end.
    ///
    /// A pass that planned no records reports no figure at all: a requirement
    /// of zero would read as "any volume fits", which is the false reassurance
    /// this exists to remove. Unknown is reported as unknown.
    fn explain(&self, error: SampleError, records_written: u64) -> SampleError {
        if !is_storage_full(&error) {
            return error;
        }

        let required_bytes = self.expected_records.saturating_mul(self.bytes_per_record);
        if required_bytes == 0 {
            return error;
        }

        SampleError::StorageFull {
            required_bytes,
            records_written,
            records_expected: self.expected_records,
            source: Box::new(error),
        }
    }
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;

    use super::*;
    use crate::corpus::{CorpusError, RecordShape};
    use crate::exit::{code_for, STORAGE_FULL};
    use crate::manifest::CallerMetadata;
    use crate::sample::SampleRate;

    /// A three-value record: two inputs and one output, twelve bytes wide.
    const RECORD_BYTES: u64 = 12;

    /// A request over `sources` at `rate`, with everything else defaulted.
    fn request(sources: &Path, rate: f64) -> SampleRequest {
        SampleRequest {
            source: sources.to_path_buf(),
            output: sources.join("derived"),
            shape: RecordShape::new(2, 1).expect("a two-input, one-output shape"),
            rate: SampleRate::new(rate).expect("a rate inside (0, 1]"),
            seed: Some(7),
            metadata: CallerMetadata::default(),
        }
    }

    /// A directory holding one corpus file of `records` whole records.
    fn corpus(label: &str, records: u64) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "refinery-output-plan-{label}-{}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).expect("create the corpus directory");
        let bytes = vec![0_u8; (records * RECORD_BYTES) as usize];
        fs::write(directory.join("shard-1.bin"), &bytes).expect("write the corpus file");
        directory
    }

    /// The out-of-space failure a write raises when the volume is full.
    fn out_of_space() -> SampleError {
        SampleError::Corpus(CorpusError::Io {
            path: PathBuf::from("trainData-binary-sampler/sample-5.bin"),
            source: io::Error::from_raw_os_error(28),
        })
    }

    #[test]
    fn a_full_volume_reports_the_bytes_another_attempt_needs() {
        let directory = corpus("another-attempt", 1_000);
        let sources = vec![directory.join("shard-1.bin")];
        let plan = OutputPlan::new(&sources, &request(&directory, 0.5)).expect("plan the output");

        // 1 000 source records at a rate of 0.5 expect 500 written. 200 are on
        // disk when the volume fills, but nothing resumes them: the next
        // attempt writes all 500 again, so the figure is the whole corpus —
        // 6 000 bytes, not the 3 600 the remainder would claim.
        let error = plan.explain(out_of_space(), 200);

        match error {
            SampleError::StorageFull {
                required_bytes,
                records_written,
                records_expected,
                ..
            } => {
                assert_eq!(required_bytes, 500 * RECORD_BYTES);
                assert_eq!(records_written, 200);
                assert_eq!(records_expected, 500);
            }
            other => panic!("a full volume must report what another attempt needs: {other}"),
        }

        fs::remove_dir_all(&directory).expect("remove the corpus directory");
    }

    #[test]
    fn the_figure_does_not_shrink_as_the_pass_gets_further() {
        // The GRQ-19 shape: a pass that died 60% of the way through needs the
        // whole corpus again, not the 40% it had left. Reporting the remainder
        // is what approved a retry onto a volume that could not hold it.
        let directory = corpus("no-shrink", 10_000);
        let sources = vec![directory.join("shard-1.bin")];
        let plan = OutputPlan::new(&sources, &request(&directory, 1.0)).expect("plan the output");

        let early = plan.explain(out_of_space(), 10);
        let late = plan.explain(out_of_space(), 6_000);

        let required = |error: &SampleError| match error {
            SampleError::StorageFull { required_bytes, .. } => *required_bytes,
            other => panic!("a full volume must carry the figure: {other}"),
        };
        assert_eq!(required(&early), 10_000 * RECORD_BYTES);
        assert_eq!(
            required(&late),
            required(&early),
            "how far the pass got does not change what the next one must fit"
        );

        fs::remove_dir_all(&directory).expect("remove the corpus directory");
    }

    #[test]
    fn the_reported_failure_still_exits_with_the_enospc_code() {
        let directory = corpus("exit-code", 400);
        let sources = vec![directory.join("shard-1.bin")];
        let plan = OutputPlan::new(&sources, &request(&directory, 1.0)).expect("plan the output");

        let error = plan.explain(out_of_space(), 10);

        // The figures are additional, not a reclassification: a caller still
        // recognises a full volume by the exit code alone.
        assert_eq!(code_for(&error), STORAGE_FULL, "{error}");
        assert!(
            format!("{error}").contains("required_bytes=4800"),
            "the message carries the figure a caller's retry gate reads: {error}"
        );

        fs::remove_dir_all(&directory).expect("remove the corpus directory");
    }

    #[test]
    fn a_failure_that_is_not_out_of_space_is_left_alone() {
        let directory = corpus("other-failure", 100);
        let sources = vec![directory.join("shard-1.bin")];
        let plan = OutputPlan::new(&sources, &request(&directory, 1.0)).expect("plan the output");

        let error = plan.explain(
            SampleError::Io {
                path: directory.join("shard-1.bin"),
                source: io::Error::from(io::ErrorKind::PermissionDenied),
            },
            0,
        );

        assert!(
            matches!(error, SampleError::Io { .. }),
            "only a full volume asks what space is still needed: {error}"
        );

        fs::remove_dir_all(&directory).expect("remove the corpus directory");
    }

    #[test]
    fn a_pass_that_plans_no_records_reports_no_figure() {
        // An empty source corpus plans nothing, so there is no requirement to
        // report. Reporting zero would read as "any volume fits".
        let directory = corpus("nothing-planned", 0);
        let sources = vec![directory.join("shard-1.bin")];
        let plan = OutputPlan::new(&sources, &request(&directory, 1.0)).expect("plan the output");

        let error = plan.explain(out_of_space(), 0);

        assert!(
            !matches!(error, SampleError::StorageFull { .. }),
            "an unknown requirement is reported as unknown, never as zero: {error}"
        );

        fs::remove_dir_all(&directory).expect("remove the corpus directory");
    }
}
