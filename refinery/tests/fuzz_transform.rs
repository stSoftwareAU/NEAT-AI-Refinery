//! Fuzzing as a composable derived-corpus transform.
//!
//! Every test drives the public [`neat_ai_refinery::fuzz`] API against a real
//! corpus on disk and asserts on the published result, so the checks survive a
//! change of implementation.
//!
//! The two properties that matter most are asserted directly: a seed and a
//! policy reproduce a corpus exactly, and expected outputs are never perturbed
//! unless the caller asked for it.

// The shared helpers serve several test binaries; not every one uses all of them.
#[allow(dead_code)]
mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use common::{encode, TempDir};
use neat_ai_refinery::corpus::{RecordShape, SourceCorpus};
use neat_ai_refinery::fuzz::{
    fuzz, FuzzBounds, FuzzDistribution, FuzzError, FuzzMode, FuzzPolicy, FuzzRequest, FuzzTargets,
};
use neat_ai_refinery::manifest::{CallerMetadata, Manifest, MANIFEST_FILE_NAME};
use neat_ai_refinery::quantise::{quantise, QuantiseRequest, QuantiseScheme};
use neat_ai_refinery::sample::{sample, SampleRate, SampleRequest};
use neat_ai_refinery::transform::TransformError;

/// Three inputs and one output — sixteen bytes a record.
fn shape() -> RecordShape {
    RecordShape::new(3, 1).expect("valid shape")
}

/// A reproducible seed, so every test states the run it expects.
const SEED: u64 = 20_260_831;

/// Records of four values: three inputs and one expected output.
fn record_values(index: u32) -> [f32; 4] {
    let step = index as f32;
    [
        0.1 + step * 0.037,
        -1.5 + step * 0.011,
        12.5 - step * 0.25,
        // The expected output — the value a default run must never touch.
        (step % 3.0) - 1.0,
    ]
}

/// Writes `count` records into `dir/name` and returns them.
fn write_shard(dir: &Path, name: &str, count: u32) -> Vec<[f32; 4]> {
    let values: Vec<[f32; 4]> = (0..count).map(record_values).collect();
    let bytes: Vec<u8> = values.iter().flat_map(|record| encode(record)).collect();
    fs::write(dir.join(name), bytes).expect("write shard");
    values
}

/// A source directory holding one shard of `count` records.
fn source_with(root: &Path, count: u32) -> (PathBuf, Vec<[f32; 4]>) {
    let source = root.join("trainData-binary");
    fs::create_dir_all(&source).expect("create the source directory");
    let values = write_shard(&source, "shard-a.bin", count);
    (source, values)
}

/// A modest absolute Gaussian policy over the inputs — the safe default.
fn policy() -> FuzzPolicy {
    FuzzPolicy::new(
        FuzzDistribution::Gaussian,
        0.05,
        FuzzMode::Absolute,
        FuzzTargets::Inputs,
        FuzzBounds::default(),
    )
    .expect("a valid policy")
}

fn request(source: &Path, output: &Path) -> FuzzRequest {
    FuzzRequest {
        source: source.to_path_buf(),
        output: output.to_path_buf(),
        shape: shape(),
        policy: policy(),
        seed: Some(SEED),
        metadata: CallerMetadata::default(),
    }
}

/// Every file name in `dir`, sorted.
fn entries(dir: &Path) -> BTreeSet<String> {
    fs::read_dir(dir)
        .expect("read the directory")
        .map(|entry| entry.expect("read the entry").file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect()
}

/// Reads a published corpus back into decoded records.
fn read_published(path: &Path, records: u64) -> Vec<Vec<f32>> {
    let corpus = SourceCorpus::open(path, shape()).expect("open the published corpus");
    assert_eq!(corpus.record_count(), records);

    (0..records)
        .map(|index| corpus.read_record(index).expect("read the record"))
        .collect()
}

#[test]
fn publishes_every_record_in_order_under_the_distribution_name() {
    let temp = TempDir::new("fuzz-publish");
    let (source, values) = source_with(temp.path(), 24);
    let output = temp.path().join("derived");

    let outcome = fuzz(&request(&source, &output)).expect("the run succeeds");

    assert_eq!(outcome.records_read, 24);
    assert_eq!(
        outcome.records_written, outcome.records_read,
        "fuzzing perturbs records, it never drops them"
    );
    assert_eq!(outcome.output_file, output.join("fuzz-gaussian.bin"));
    assert_eq!(
        entries(&output),
        BTreeSet::from(["fuzz-gaussian.bin".into(), MANIFEST_FILE_NAME.into()]),
        "the corpus and its provenance are published together"
    );
    assert_eq!(outcome.values_perturbed, 24 * 3, "three inputs a record");
    assert_eq!(outcome.values_clamped, 0, "no bounds were configured");
    assert_eq!(outcome.values_preserved, 0, "the fixture is all finite");

    // The layout is untouched, so record n of the output still stands for
    // record n of the source and a downstream index means what it meant.
    let published = read_published(&outcome.output_file, 24);
    assert_eq!(published.len(), values.len());
    for (index, (original, perturbed)) in values.iter().zip(&published).enumerate() {
        assert_eq!(perturbed.len(), 4, "record {index} keeps its width");
        for (value, moved) in original.iter().take(3).zip(perturbed) {
            // A modest absolute policy moves a value, but not far.
            assert!(
                (moved - value).abs() < 1.0,
                "record {index}: {value} moved to {moved}"
            );
        }
    }
}

#[test]
fn leaves_expected_outputs_untouched_by_default() {
    let temp = TempDir::new("fuzz-outputs-untouched");
    let (source, values) = source_with(temp.path(), 64);
    let output = temp.path().join("derived");

    let outcome = fuzz(&request(&source, &output)).expect("the run succeeds");
    let published = read_published(&outcome.output_file, 64);

    let mut inputs_moved = 0_u32;
    for (index, (original, perturbed)) in values.iter().zip(&published).enumerate() {
        assert_eq!(
            perturbed[3], original[3],
            "record {index}: the expected output must survive bit for bit"
        );
        for value in 0..3 {
            if perturbed[value] != original[value] {
                inputs_moved += 1;
            }
        }
    }

    assert!(
        inputs_moved > 0,
        "the run must actually have perturbed the inputs"
    );
    assert_eq!(
        outcome.manifest.transform.parameters["targets"], "inputs",
        "the default is recorded, not merely applied"
    );
}

#[test]
fn perturbs_outputs_only_when_explicitly_requested() {
    let temp = TempDir::new("fuzz-targets");
    let (source, values) = source_with(temp.path(), 64);

    let mut all = request(&source, &temp.path().join("all"));
    all.policy = FuzzPolicy::new(
        FuzzDistribution::Gaussian,
        0.05,
        FuzzMode::Absolute,
        FuzzTargets::All,
        FuzzBounds::default(),
    )
    .expect("a valid policy");
    let all = fuzz(&all).expect("the run succeeds");

    let mut outputs_only = request(&source, &temp.path().join("outputs"));
    outputs_only.policy = FuzzPolicy::new(
        FuzzDistribution::Gaussian,
        0.05,
        FuzzMode::Absolute,
        FuzzTargets::Outputs,
        FuzzBounds::default(),
    )
    .expect("a valid policy");
    let outputs_only = fuzz(&outputs_only).expect("the run succeeds");

    assert_eq!(all.values_perturbed, 64 * 4, "every value in every record");
    assert_eq!(outputs_only.values_perturbed, 64, "one output a record");

    let published_all = read_published(&all.output_file, 64);
    let published_outputs = read_published(&outputs_only.output_file, 64);

    let mut outputs_moved = 0_u32;
    for (index, original) in values.iter().enumerate() {
        // `all` reaches the expected output; `outputs` leaves every input alone.
        if published_all[index][3] != original[3] {
            outputs_moved += 1;
        }
        for value in 0..3 {
            assert_eq!(
                published_outputs[index][value], original[value],
                "record {index}: --targets outputs must not touch an input"
            );
        }
    }

    assert!(
        outputs_moved > 0,
        "--targets all must reach the expected outputs"
    );
}

#[test]
fn the_same_seed_and_policy_produce_an_identical_derived_corpus() {
    let temp = TempDir::new("fuzz-reproducible");
    let (source, _) = source_with(temp.path(), 128);

    let first = fuzz(&request(&source, &temp.path().join("first"))).expect("the first run");
    let second = fuzz(&request(&source, &temp.path().join("second"))).expect("the second run");

    assert_eq!(
        fs::read(&first.output_file).expect("read the first corpus"),
        fs::read(&second.output_file).expect("read the second corpus"),
        "the same seed and policy must produce the same bytes"
    );
    assert_eq!(
        first.manifest.output.checksum.value, second.manifest.output.checksum.value,
        "and the checksums must agree"
    );
    assert_eq!(first.seed, SEED);
    assert_eq!(first.manifest.transform.seed, Some(SEED));
}

#[test]
fn a_different_seed_produces_a_different_corpus() {
    let temp = TempDir::new("fuzz-seed-matters");
    let (source, _) = source_with(temp.path(), 128);

    let first = fuzz(&request(&source, &temp.path().join("first"))).expect("the first run");
    let mut other = request(&source, &temp.path().join("other"));
    other.seed = Some(SEED + 1);
    let other = fuzz(&other).expect("the second run");

    assert_ne!(
        fs::read(&first.output_file).expect("read the first corpus"),
        fs::read(&other.output_file).expect("read the second corpus"),
        "a different seed must draw different noise"
    );
}

#[test]
fn a_policy_change_alone_produces_a_different_corpus() {
    let temp = TempDir::new("fuzz-policy-matters");
    let (source, _) = source_with(temp.path(), 128);

    let gaussian = fuzz(&request(&source, &temp.path().join("gaussian"))).expect("the first run");

    let mut uniform = request(&source, &temp.path().join("uniform"));
    uniform.policy = FuzzPolicy::new(
        FuzzDistribution::Uniform,
        0.05,
        FuzzMode::Absolute,
        FuzzTargets::Inputs,
        FuzzBounds::default(),
    )
    .expect("a valid policy");
    let uniform = fuzz(&uniform).expect("the second run");

    assert_eq!(
        uniform.output_file,
        temp.path().join("uniform/fuzz-uniform.bin")
    );
    assert_ne!(
        fs::read(&gaussian.output_file).expect("read the gaussian corpus"),
        fs::read(&uniform.output_file).expect("read the uniform corpus"),
        "the same seed under a different distribution is a different corpus"
    );
}

#[test]
fn seeds_from_the_operating_system_and_reports_the_seed_it_used() {
    let temp = TempDir::new("fuzz-os-seed");
    let (source, _) = source_with(temp.path(), 32);

    let mut unseeded = request(&source, &temp.path().join("unseeded"));
    unseeded.seed = None;
    let outcome = fuzz(&unseeded).expect("an unseeded run succeeds");

    assert_eq!(
        outcome.manifest.transform.seed,
        Some(outcome.seed),
        "the seed actually used is always recorded"
    );

    // Replaying the reported seed must reproduce the corpus byte for byte.
    let mut replay = request(&source, &temp.path().join("replay"));
    replay.seed = Some(outcome.seed);
    let replayed = fuzz(&replay).expect("the replay succeeds");

    assert_eq!(
        fs::read(&outcome.output_file).expect("read the first corpus"),
        fs::read(&replayed.output_file).expect("read the replayed corpus"),
        "a reported seed is enough to replay the run"
    );
}

#[test]
fn records_the_exact_perturbation_policy_in_the_manifest() {
    let temp = TempDir::new("fuzz-manifest");
    let (source, _) = source_with(temp.path(), 8);
    let output = temp.path().join("derived");
    let mut request = request(&source, &output);
    request.policy = FuzzPolicy::new(
        FuzzDistribution::Uniform,
        0.25,
        FuzzMode::Relative,
        FuzzTargets::All,
        FuzzBounds::new(Some(-8.0), Some(8.0)).expect("valid bounds"),
    )
    .expect("a valid policy");
    request.metadata =
        CallerMetadata::parse(&["grq_observation_version=42".to_string()]).expect("valid metadata");

    let outcome = fuzz(&request).expect("the run succeeds");
    let manifest = Manifest::load(&outcome.manifest_file).expect("read the published manifest");

    assert_eq!(manifest.transform.name, "fuzz");
    assert_eq!(manifest.transform.seed, Some(SEED));
    let parameters = &manifest.transform.parameters;
    assert_eq!(parameters["distribution"], "uniform");
    assert_eq!(parameters["scale"], 0.25);
    assert_eq!(parameters["mode"], "relative");
    assert_eq!(parameters["targets"], "all");
    assert_eq!(parameters["clamp_min"], -8.0);
    assert_eq!(parameters["clamp_max"], 8.0);
    assert_eq!(parameters["non_finite_source"], "preserve");
    assert_eq!(parameters["non_finite_result"], "fail");

    // Fuzzing is not a representation transform: the layout is unchanged, so
    // no source layout is recorded beside it.
    assert_eq!(manifest.record_shape.encoding, "float32");
    assert_eq!(manifest.record_shape.bytes_per_record, 16);
    assert_eq!(manifest.source_record_shape, None);

    assert_eq!(manifest.source.record_count, 8);
    assert_eq!(manifest.output.record_count, 8);
    assert_eq!(manifest.output.bytes, 8 * 16);
    assert_eq!(manifest.output.file, "fuzz-uniform.bin");
    assert_eq!(manifest.metadata.get("grq_observation_version"), Some("42"));
}

#[test]
fn records_absent_bounds_explicitly_rather_than_omitting_them() {
    let temp = TempDir::new("fuzz-manifest-no-bounds");
    let (source, _) = source_with(temp.path(), 4);

    let outcome = fuzz(&request(&source, &temp.path().join("derived"))).expect("the run succeeds");
    let parameters = &outcome.manifest.transform.parameters;

    assert!(parameters["clamp_min"].is_null(), "{parameters:?}");
    assert!(parameters["clamp_max"].is_null(), "{parameters:?}");
}

#[test]
fn clamps_every_published_value_into_the_configured_bounds() {
    let temp = TempDir::new("fuzz-bounds");
    let (source, _) = source_with(temp.path(), 64);
    let output = temp.path().join("derived");
    let mut request = request(&source, &output);
    // A scale far larger than the corpus, so the bounds are the only thing
    // holding the values in — every perturbed value must land on one.
    request.policy = FuzzPolicy::new(
        FuzzDistribution::Uniform,
        1000.0,
        FuzzMode::Absolute,
        FuzzTargets::Inputs,
        FuzzBounds::new(Some(-1.0), Some(1.0)).expect("valid bounds"),
    )
    .expect("a valid policy");

    let outcome = fuzz(&request).expect("the run succeeds");
    let published = read_published(&outcome.output_file, 64);

    for (index, record) in published.iter().enumerate() {
        for value in record.iter().take(3) {
            assert!(
                (-1.0..=1.0).contains(value),
                "record {index}: {value} escaped the bounds"
            );
        }
    }
    assert!(
        outcome.values_clamped > 0,
        "the fixture must actually exercise clamping"
    );
    assert!(outcome.values_clamped <= outcome.values_perturbed);
}

#[test]
fn honours_a_one_sided_bound() {
    let temp = TempDir::new("fuzz-one-sided-bound");
    let (source, _) = source_with(temp.path(), 64);
    let mut request = request(&source, &temp.path().join("derived"));
    request.policy = FuzzPolicy::new(
        FuzzDistribution::Uniform,
        1000.0,
        FuzzMode::Absolute,
        FuzzTargets::Inputs,
        FuzzBounds::new(Some(0.0), None).expect("valid bounds"),
    )
    .expect("a valid policy");

    let outcome = fuzz(&request).expect("the run succeeds");
    let published = read_published(&outcome.output_file, 64);

    let mut above = 0_u32;
    for record in &published {
        for value in record.iter().take(3) {
            assert!(*value >= 0.0, "{value} fell below the floor");
            if *value > 1.0 {
                above += 1;
            }
        }
    }
    assert!(above > 0, "an absent ceiling must not cap anything");
}

#[test]
fn preserves_a_non_finite_source_value_rather_than_perturbing_it() {
    let temp = TempDir::new("fuzz-non-finite-source");
    let source = temp.path().join("trainData-binary");
    fs::create_dir_all(&source).expect("create the source directory");
    let records: [[f32; 4]; 3] = [
        [f32::NAN, 1.0, 2.0, 0.5],
        [f32::INFINITY, f32::NEG_INFINITY, 3.0, -0.5],
        [4.0, 5.0, 6.0, 0.0],
    ];
    let bytes: Vec<u8> = records.iter().flat_map(|record| encode(record)).collect();
    fs::write(source.join("shard-a.bin"), bytes).expect("write the shard");

    let outcome = fuzz(&request(&source, &temp.path().join("derived"))).expect("the run succeeds");
    let published = read_published(&outcome.output_file, 3);

    assert!(published[0][0].is_nan(), "a NaN input is left as it was");
    assert_eq!(published[1][0], f32::INFINITY);
    assert_eq!(published[1][1], f32::NEG_INFINITY);
    assert_eq!(
        outcome.values_preserved, 3,
        "each non-finite value is counted, never silently dropped"
    );
    assert_eq!(
        outcome.values_perturbed, 6,
        "the six finite inputs are the ones that moved"
    );
    for (index, record) in records.iter().enumerate() {
        assert_eq!(
            published[index][3], record[3],
            "record {index}: the expected output is still untouched"
        );
    }
}

#[test]
fn fails_loud_when_a_perturbation_leaves_the_finite_range() {
    let temp = TempDir::new("fuzz-overflow");
    let source = temp.path().join("trainData-binary");
    fs::create_dir_all(&source).expect("create the source directory");
    let bytes = encode(&[1.0, 2.0, 3.0, 0.5]);
    fs::write(source.join("shard-a.bin"), bytes).expect("write the shard");
    let output = temp.path().join("derived");

    let mut request = request(&source, &output);
    // A scale this far beyond the `f32` range cannot produce a storable value,
    // and the configured ceiling must not disguise the overflow as one.
    request.policy = FuzzPolicy::new(
        FuzzDistribution::Uniform,
        1.0e300,
        FuzzMode::Absolute,
        FuzzTargets::Inputs,
        FuzzBounds::new(None, Some(1.0)).expect("valid bounds"),
    )
    .expect("a valid policy");

    let error = fuzz(&request).expect_err("an unrepresentable result is fatal");

    assert!(
        matches!(
            error,
            FuzzError::NonFiniteResult {
                record: 0,
                value: 0,
                ..
            }
        ),
        "{error:?}"
    );
    assert!(error.to_string().contains("record 0"), "{error}");
    assert!(!output.exists(), "nothing is published");
}

#[test]
fn rejects_a_scale_that_is_not_a_positive_finite_number() {
    for scale in [0.0, -0.5, f64::NAN, f64::INFINITY] {
        let error = FuzzPolicy::new(
            FuzzDistribution::Gaussian,
            scale,
            FuzzMode::Absolute,
            FuzzTargets::Inputs,
            FuzzBounds::default(),
        )
        .expect_err("a scale of {scale} is not a perturbation");

        assert!(matches!(error, FuzzError::InvalidScale { .. }), "{error:?}");
    }
}

#[test]
fn rejects_bounds_that_cross_or_are_not_finite() {
    for (min, max) in [
        (Some(1.0_f32), Some(-1.0_f32)),
        (Some(f32::NAN), None),
        (None, Some(f32::INFINITY)),
    ] {
        let error = FuzzBounds::new(min, max).expect_err("the bounds are unusable");

        assert!(
            matches!(error, FuzzError::InvalidBounds { .. }),
            "{error:?}"
        );
    }

    // Equal bounds are a degenerate but well-defined policy: pin every
    // perturbed value to one number.
    FuzzBounds::new(Some(1.0), Some(1.0)).expect("equal bounds are allowed");
}

#[test]
fn leaves_the_source_corpus_byte_for_byte_unchanged() {
    let temp = TempDir::new("fuzz-immutable-source");
    let (source, _) = source_with(temp.path(), 32);
    let shard = source.join("shard-a.bin");
    let before = fs::read(&shard).expect("read the source");

    fuzz(&request(&source, &temp.path().join("derived"))).expect("the run succeeds");

    assert_eq!(
        fs::read(&shard).expect("read the source again"),
        before,
        "the source corpus is never written to"
    );
    assert_eq!(entries(&source), BTreeSet::from(["shard-a.bin".into()]));
}

#[test]
fn composes_with_sampling_over_the_published_corpus() {
    let temp = TempDir::new("fuzz-composes");
    let (source, _) = source_with(temp.path(), 200);
    let sampled = temp.path().join("sampled");
    let fuzzed = temp.path().join("sampled-fuzzed");

    let sample_outcome = sample(&SampleRequest {
        source: source.clone(),
        output: sampled.clone(),
        shape: shape(),
        rate: SampleRate::new(0.25).expect("valid rate"),
        seed: Some(SEED),
        metadata: CallerMetadata::default(),
    })
    .expect("the sampling run succeeds");

    let outcome = fuzz(&request(&sampled, &fuzzed)).expect("the fuzzing run succeeds");

    assert_eq!(
        outcome.records_read, sample_outcome.records_written,
        "the second transform reads exactly what the first published"
    );
    assert_eq!(outcome.records_written, outcome.records_read);
    assert_eq!(
        outcome.sources,
        vec![sampled.join(sample_outcome.manifest.output.file.clone())],
        "the manifest beside the corpus is not mistaken for records"
    );
    assert!(
        sampled.join("sample-25.bin").exists(),
        "the sampled corpus survives being fuzzed"
    );
}

#[test]
fn refuses_a_source_whose_manifest_declares_another_encoding() {
    let temp = TempDir::new("fuzz-encoding-mismatch");
    let (source, _) = source_with(temp.path(), 16);
    let quantised = temp.path().join("quantised");

    quantise(&QuantiseRequest {
        source,
        output: quantised.clone(),
        shape: shape(),
        scheme: QuantiseScheme::BFloat16,
        metadata: CallerMetadata::default(),
    })
    .expect("the quantisation run succeeds");

    let output = temp.path().join("derived");
    let error = fuzz(&request(&quantised, &output))
        .expect_err("bfloat16 bytes must not be read as float32");

    assert!(
        matches!(error, FuzzError::SourceEncodingMismatch { ref found, .. } if found == "bfloat16"),
        "{error:?}"
    );
    assert!(!output.exists(), "nothing is published");
}

#[test]
fn refuses_a_record_shape_the_source_manifest_contradicts() {
    let temp = TempDir::new("fuzz-width-mismatch");
    let (source, _) = source_with(temp.path(), 12);
    let sampled = temp.path().join("sampled");
    sample(&SampleRequest {
        source,
        output: sampled.clone(),
        shape: shape(),
        rate: SampleRate::new(1.0).expect("valid rate"),
        seed: Some(7),
        metadata: CallerMetadata::default(),
    })
    .expect("the sampling run succeeds");

    let mut request = request(&sampled, &temp.path().join("derived"));
    // Six values a record, not four: the records would be split in the wrong
    // places, and the noise would land on the wrong values.
    request.shape = RecordShape::new(5, 1).expect("valid shape");

    let error = fuzz(&request).expect_err("the declared width is checked");

    assert!(
        matches!(
            error,
            FuzzError::SourceWidthMismatch {
                expected: 24,
                found: 16,
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn fails_loud_on_a_source_manifest_it_cannot_read() {
    let temp = TempDir::new("fuzz-broken-manifest");
    let (source, _) = source_with(temp.path(), 4);
    fs::write(source.join(MANIFEST_FILE_NAME), b"{ not a manifest").expect("write the fixture");
    let output = temp.path().join("derived");

    let error = fuzz(&request(&source, &output)).expect_err("a broken manifest is fatal");

    assert!(
        matches!(error, FuzzError::Transform(TransformError::Manifest(_))),
        "{error:?}"
    );
    assert!(!output.exists(), "nothing is published");
}

#[test]
fn refuses_an_output_directory_that_overlaps_the_source() {
    let temp = TempDir::new("fuzz-overlap");
    let (source, _) = source_with(temp.path(), 4);

    let error = fuzz(&request(&source, &source.join("inside")))
        .expect_err("publishing inside the source would delete it");

    assert!(
        matches!(
            error,
            FuzzError::Transform(TransformError::OverlappingCorpora { .. })
        ),
        "{error:?}"
    );
}

#[test]
fn refuses_a_source_directory_holding_no_corpus_files() {
    let temp = TempDir::new("fuzz-empty-source");
    let source = temp.path().join("trainData-binary");
    fs::create_dir_all(&source).expect("create the source directory");
    fs::write(source.join("notes.txt"), b"not a corpus").expect("write the note");

    let error = fuzz(&request(&source, &temp.path().join("derived")))
        .expect_err("there is nothing to fuzz");

    assert!(
        matches!(
            error,
            FuzzError::Transform(TransformError::NoCorpusFiles { .. })
        ),
        "{error:?}"
    );
}

#[test]
fn fails_loud_on_a_partial_trailing_record() {
    let temp = TempDir::new("fuzz-partial-record");
    let source = temp.path().join("trainData-binary");
    fs::create_dir_all(&source).expect("create the source directory");
    let mut bytes = encode(&[1.0, 2.0, 3.0, 4.0]);
    bytes.extend_from_slice(&[0_u8; 5]);
    fs::write(source.join("shard-a.bin"), bytes).expect("write the shard");
    let output = temp.path().join("derived");

    let error = fuzz(&request(&source, &output)).expect_err("a partial record is fatal");

    assert!(
        matches!(error, FuzzError::Transform(TransformError::Corpus(_))),
        "{error:?}"
    );
    assert!(
        !output.exists(),
        "a failed run publishes nothing and leaves no scratch"
    );
    assert_eq!(
        entries(temp.path()),
        BTreeSet::from(["trainData-binary".into()])
    );
}

#[test]
fn replaces_a_previously_published_corpus_whole() {
    let temp = TempDir::new("fuzz-republish");
    let (source, _) = source_with(temp.path(), 8);
    let output = temp.path().join("derived");
    fs::create_dir_all(&output).expect("create the live directory");
    fs::write(output.join("stale.bin"), b"stale").expect("write the stale corpus");

    fuzz(&request(&source, &output)).expect("the run succeeds");

    assert_eq!(
        entries(&output),
        BTreeSet::from(["fuzz-gaussian.bin".into(), MANIFEST_FILE_NAME.into()]),
        "the live directory is replaced whole, not merged into"
    );
}

#[test]
fn keeps_uniform_absolute_noise_inside_the_scale() {
    let temp = TempDir::new("fuzz-uniform-range");
    let (source, values) = source_with(temp.path(), 128);
    let mut request = request(&source, &temp.path().join("derived"));
    request.policy = FuzzPolicy::new(
        FuzzDistribution::Uniform,
        0.5,
        FuzzMode::Absolute,
        FuzzTargets::Inputs,
        FuzzBounds::default(),
    )
    .expect("a valid policy");

    let outcome = fuzz(&request).expect("the run succeeds");
    let published = read_published(&outcome.output_file, 128);

    // Uniform noise is bounded by the scale itself, which is what makes it the
    // choice when a hard perturbation limit matters more than a tail.
    let mut widest = 0.0_f32;
    for (index, (original, perturbed)) in values.iter().zip(&published).enumerate() {
        for (value, moved) in original.iter().take(3).zip(perturbed) {
            let offset = (moved - value).abs();
            assert!(
                offset <= 0.5 + 1.0e-6,
                "record {index}: {value} moved {offset}, beyond the 0.5 scale"
            );
            widest = widest.max(offset);
        }
    }
    assert!(widest > 0.25, "the fixture must exercise the whole range");
}
