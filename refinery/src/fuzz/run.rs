//! The fuzzing run itself.

use std::path::{Path, PathBuf};

use rand::RngExt;

use super::noise::NoiseSource;
use super::{FuzzError, FuzzRequest};
use crate::corpus::{DerivedDestination, RecordReader, RecordWriter};
use crate::manifest::{
    Checksum, Manifest, OutputArtefact, SourceIdentity, TransformRecord, MANIFEST_FILE_NAME,
};
use crate::transform::{
    corpus_files, file_bytes, resolved_source, source_file, source_manifest, source_manifest_path,
    StagedCorpus,
};

/// The transform name recorded in the manifest.
const TRANSFORM_NAME: &str = "fuzz";

/// What a completed fuzzing run produced.
#[derive(Debug, Clone)]
pub struct FuzzOutcome {
    /// The corpus files read, in discovery order.
    pub sources: Vec<PathBuf>,
    /// Records read across every source.
    pub records_read: u64,
    /// Records written — always equal to [`FuzzOutcome::records_read`], because
    /// fuzzing perturbs records rather than selecting them.
    pub records_written: u64,
    /// Targeted, finite values that noise was applied to.
    pub values_perturbed: u64,
    /// Perturbed values a bound had to move.
    pub values_clamped: u64,
    /// Targeted values that were not finite in the source and were therefore
    /// written back exactly as they were.
    pub values_preserved: u64,
    /// The published corpus file.
    pub output_file: PathBuf,
    /// The seed the run used — supplied, or drawn from the operating system.
    pub seed: u64,
    /// The published manifest file.
    pub manifest_file: PathBuf,
    /// The provenance record published beside the corpus.
    pub manifest: Manifest,
}

/// Perturbs the source corpus under the requested policy and publishes the
/// result as a fresh derived corpus.
///
/// The source is only ever read. Records keep their order, their count and
/// their layout — only the targeted values move — and the derived corpus is
/// built in a staging directory, manifest included, and published with an
/// atomic rename.
///
/// # Errors
///
/// Returns [`FuzzError::SourceEncodingMismatch`] or
/// [`FuzzError::SourceWidthMismatch`] when the source's own manifest
/// contradicts what the run was asked to read, [`FuzzError::NonFiniteResult`]
/// when a perturbation leaves the finite range, and [`FuzzError::Transform`]
/// for a missing corpus, an overlapping destination, a malformed record, a
/// failed write, provenance that cannot be produced, or a failed publish.
pub fn fuzz(request: &FuzzRequest) -> Result<FuzzOutcome, FuzzError> {
    let source = &request.source;
    let resolved = resolved_source(source, &request.output)?;
    let shape = request.shape;
    check_source_declaration(source, request)?;

    let sources = corpus_files(source)?;
    let mut read_files = Vec::with_capacity(sources.len());
    for path in &sources {
        read_files.push(source_file(path)?);
    }

    let seed = request.seed.unwrap_or_else(|| rand::rng().random());
    let mut noise = NoiseSource::new(request.policy.distribution(), seed);

    let staged = StagedCorpus::create(&request.output)?;
    let file_name = request.policy.distribution().file_name();
    let destination = DerivedDestination::new(staged.path().join(&file_name), &sources)?;
    let mut writer = RecordWriter::create(&destination, shape)?;

    // Two scratch buffers, reused: the working set stays the reader's buffer
    // plus a single record however large the corpus is.
    let mut values: Vec<f32> = Vec::with_capacity(shape.record_values());
    let mut encoded: Vec<u8> = Vec::with_capacity(shape.bytes_per_record());
    let mut counts = ValueCounts::default();

    let mut reader = RecordReader::open(&sources, shape)?;
    let mut index = 0_u64;
    while let Some(record) = reader.next_record() {
        shape.encoding().decode_into(record?, &mut values);
        perturb_record(request, &mut noise, index, &mut values, &mut counts)?;

        encoded.clear();
        for value in &values {
            shape.encoding().encode_into(*value, &mut encoded);
        }
        writer.write_record(&encoded)?;
        index += 1;
    }
    let records_read = reader.records_read();
    let records_written = writer.finish()?;

    let staged_file = staged.path().join(&file_name);
    let manifest = Manifest::new(
        TransformRecord::new(TRANSFORM_NAME, request.policy.parameters(), Some(seed)),
        // Fuzzing edits values in place: both corpora share one layout, so no
        // source layout is recorded beside it.
        shape.into(),
        SourceIdentity::new(resolved, read_files, records_read),
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
    let manifest_file = staged.destination().join(MANIFEST_FILE_NAME);
    staged.publish()?;

    Ok(FuzzOutcome {
        sources,
        records_read,
        records_written,
        values_perturbed: counts.perturbed,
        values_clamped: counts.clamped,
        values_preserved: counts.preserved,
        output_file,
        seed,
        manifest_file,
        manifest,
    })
}

/// How many values a run perturbed, clamped and preserved.
#[derive(Debug, Default)]
struct ValueCounts {
    perturbed: u64,
    clamped: u64,
    preserved: u64,
}

/// Applies the policy to one decoded record, in place.
///
/// One draw is taken for every targeted value, whether or not that value can be
/// perturbed, so the noise sequence depends on the policy and the record shape
/// alone — never on the values a corpus happens to hold.
fn perturb_record(
    request: &FuzzRequest,
    noise: &mut NoiseSource,
    record: u64,
    values: &mut [f32],
    counts: &mut ValueCounts,
) -> Result<(), FuzzError> {
    let policy = &request.policy;
    for (value, original) in values.iter_mut().enumerate() {
        if !policy.targets().includes(value, &request.shape) {
            continue;
        }

        let perturbed = policy.perturb(*original, noise.draw());
        if perturbed.preserved {
            counts.preserved += 1;
            continue;
        }
        if !perturbed.value.is_finite() {
            return Err(FuzzError::NonFiniteResult {
                record,
                value,
                original: *original,
                perturbed: perturbed.value,
            });
        }

        counts.perturbed += 1;
        counts.clamped += u64::from(perturbed.clamped);
        *original = perturbed.value;
    }
    Ok(())
}

/// Checks the source's own manifest, when it has one, against what this run was
/// told to read.
///
/// A Refinery-published corpus carries its layout beside it, so composing
/// transforms need not take the caller's word for it. Fuzzing a quantised
/// corpus as if it were `float32`, or reading one at the wrong record width, is
/// caught here rather than scattering noise across reinterpreted bytes. A
/// source with no manifest — a raw training corpus — is read as the caller
/// described it.
fn check_source_declaration(source: &Path, request: &FuzzRequest) -> Result<(), FuzzError> {
    let Some(manifest) = source_manifest(source)? else {
        return Ok(());
    };
    let path = source_manifest_path(source);
    let expected = request.shape.encoding();

    if manifest.record_shape.encoding != expected.name() {
        return Err(FuzzError::SourceEncodingMismatch {
            manifest: path,
            expected: expected.name().to_string(),
            found: manifest.record_shape.encoding,
        });
    }
    if manifest.record_shape.bytes_per_record != request.shape.bytes_per_record() {
        return Err(FuzzError::SourceWidthMismatch {
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
    use crate::corpus::RecordShape;
    use crate::fuzz::{FuzzBounds, FuzzDistribution, FuzzMode, FuzzPolicy, FuzzTargets};
    use crate::manifest::CallerMetadata;

    fn request(targets: FuzzTargets, bounds: FuzzBounds) -> FuzzRequest {
        FuzzRequest {
            source: PathBuf::from("source"),
            output: PathBuf::from("derived"),
            shape: RecordShape::new(2, 1).expect("valid shape"),
            policy: FuzzPolicy::new(
                FuzzDistribution::Uniform,
                1.0,
                FuzzMode::Absolute,
                targets,
                bounds,
            )
            .expect("a valid policy"),
            seed: Some(1),
            metadata: CallerMetadata::default(),
        }
    }

    #[test]
    fn perturbs_only_the_targeted_values_of_a_record() {
        let request = request(FuzzTargets::Inputs, FuzzBounds::default());
        let mut noise = NoiseSource::new(FuzzDistribution::Uniform, 1);
        let mut values = [1.0_f32, 2.0, 3.0];
        let mut counts = ValueCounts::default();

        perturb_record(&request, &mut noise, 0, &mut values, &mut counts).expect("the record");

        assert_eq!(counts.perturbed, 2, "two inputs, one untouched output");
        assert_eq!(
            values[2], 3.0,
            "the expected output is left exactly as it was"
        );
        assert_ne!(values[0], 1.0);
    }

    #[test]
    fn counts_preserved_and_clamped_values_separately() {
        let bounds = FuzzBounds::new(Some(0.0), Some(0.0)).expect("valid bounds");
        let request = request(FuzzTargets::All, bounds);
        let mut noise = NoiseSource::new(FuzzDistribution::Uniform, 1);
        let mut values = [f32::NAN, 5.0, 5.0];
        let mut counts = ValueCounts::default();

        perturb_record(&request, &mut noise, 0, &mut values, &mut counts).expect("the record");

        assert_eq!(counts.preserved, 1);
        assert_eq!(counts.perturbed, 2);
        assert_eq!(counts.clamped, 2, "both finite values were pinned");
        assert!(values[0].is_nan());
        assert_eq!(values[1], 0.0);
    }

    #[test]
    fn names_the_record_and_value_a_perturbation_overflowed_in() {
        let mut request = request(FuzzTargets::Outputs, FuzzBounds::default());
        // A scale this far beyond the `f32` range overflows whatever is drawn,
        // so the failure is the policy's rather than the seed's.
        request.policy = FuzzPolicy::new(
            FuzzDistribution::Uniform,
            1.0e300,
            FuzzMode::Absolute,
            FuzzTargets::Outputs,
            FuzzBounds::default(),
        )
        .expect("a valid policy");
        let mut noise = NoiseSource::new(FuzzDistribution::Uniform, 1);
        let mut values = [1.0_f32, 2.0, 3.0];
        let mut counts = ValueCounts::default();

        let error = perturb_record(&request, &mut noise, 9, &mut values, &mut counts)
            .expect_err("an unrepresentable result is fatal");

        assert!(
            matches!(
                error,
                FuzzError::NonFiniteResult {
                    record: 9,
                    value: 2,
                    original,
                    ..
                } if original == 3.0
            ),
            "{error:?}"
        );
        assert_eq!(counts.perturbed, 0, "the inputs were never targeted");
    }
}
