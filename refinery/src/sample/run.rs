//! The sampling run itself.

use std::fs;
use std::path::{Path, PathBuf};

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{RngExt, SeedableRng};

use super::{SampleError, SampleRequest, StagedCorpus};
use crate::corpus::{discover_sources, DerivedDestination, RecordReader, RecordWriter};

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
}

/// Samples the source corpus into a freshly published derived corpus.
///
/// The source is only ever read. The derived corpus is built in a staging
/// directory and published with an atomic rename, so the live directory is
/// replaced whole or not at all.
///
/// # Errors
///
/// Returns [`SampleError::NoCorpusFiles`] for a source directory with no
/// `.bin` files, [`SampleError::OutputInsideSource`] when the derived corpus
/// would land on or inside the source, [`SampleError::Corpus`] for a malformed
/// record or a failed write, [`SampleError::Publish`] when the swap fails, and
/// [`SampleError::Io`] for any other filesystem failure.
pub fn sample(request: &SampleRequest) -> Result<SampleOutcome, SampleError> {
    let source = &request.source;
    check_separation(source, &request.output)?;

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
    for path in &sources {
        records_read += sample_file(path, request, &mut rng, &mut writer)?;
    }
    let records_written = writer.finish()?;

    let output_file = staged.destination().join(&file_name);
    staged.publish()?;

    Ok(SampleOutcome {
        sources,
        records_read,
        records_written,
        output_file,
        seed,
    })
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

/// The `.bin` corpus files in `source`, in discovery order.
///
/// Discovery is non-recursive and skips dot-files; this narrows it to the
/// `.bin` files the sampler reads, so a stray note or checksum beside the
/// corpus is not mistaken for records.
fn corpus_files(source: &Path) -> Result<Vec<PathBuf>, SampleError> {
    let files: Vec<PathBuf> = discover_sources(source)?
        .into_iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "bin"))
        .collect();

    if files.is_empty() {
        return Err(SampleError::NoCorpusFiles {
            path: source.to_path_buf(),
        });
    }
    Ok(files)
}

/// Rejects an output directory that resolves onto or inside the source.
///
/// Publishing renames the whole output directory, so an output inside the
/// source would put a source corpus one rename away from deletion. Resolving
/// both paths first means a relative path, a `..` segment or a symlink cannot
/// hide the overlap.
fn check_separation(source: &Path, output: &Path) -> Result<(), SampleError> {
    let resolved_source = fs::canonicalize(source).map_err(|e| SampleError::io(source, e))?;
    let resolved_output = resolve_output(output)?;

    if resolved_output.starts_with(&resolved_source) {
        return Err(SampleError::OutputInsideSource {
            output: resolved_output,
            source: resolved_source,
        });
    }
    Ok(())
}

/// Resolves the output directory, which need not exist yet.
///
/// An existing path is canonicalised; otherwise its parent is, and the name is
/// re-joined. The parent must already exist — a derived corpus under a missing
/// directory is a caller mistake worth failing on before any file is read.
fn resolve_output(output: &Path) -> Result<PathBuf, SampleError> {
    if output.exists() {
        return fs::canonicalize(output).map_err(|e| SampleError::io(output, e));
    }

    let name = output.file_name().ok_or_else(|| {
        SampleError::io(
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
    let parent = fs::canonicalize(&parent).map_err(|e| SampleError::io(&parent, e))?;

    Ok(parent.join(name))
}
