//! The sampling run itself.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{RngExt, SeedableRng};

use super::{SampleError, SampleRequest, StagedCorpus};
use crate::corpus::{discover_sources, DerivedDestination, RecordReader, RecordWriter};
use crate::manifest::{
    Checksum, Manifest, OutputArtefact, SourceFile, SourceIdentity, TransformRecord,
};

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
    let resolved_source = check_separation(source, &request.output)?;

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

/// Identifies one source file by name and byte length.
fn source_file(path: &Path) -> Result<SourceFile, SampleError> {
    Ok(SourceFile {
        name: path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
        bytes: file_bytes(path)?,
    })
}

/// The byte length of `path`.
fn file_bytes(path: &Path) -> Result<u64, SampleError> {
    Ok(fs::metadata(path)
        .map_err(|e| SampleError::io(path, e))?
        .len())
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

/// Rejects an output directory that overlaps the source corpus, returning the
/// canonical source path the manifest records.
///
/// Publishing renames the whole output directory aside and deletes it, so
/// either nesting is fatal: an output inside the source, and a source inside
/// the output, both put an immutable source corpus one rename away from
/// deletion. Resolving both paths first means a relative path, a `..` segment
/// or a symlink cannot hide the overlap.
fn check_separation(source: &Path, output: &Path) -> Result<PathBuf, SampleError> {
    let resolved_source = fs::canonicalize(source).map_err(|e| SampleError::io(source, e))?;
    let resolved_output = resolve_output(output)?;

    if resolved_output.starts_with(&resolved_source)
        || resolved_source.starts_with(&resolved_output)
    {
        return Err(SampleError::OverlappingCorpora {
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
