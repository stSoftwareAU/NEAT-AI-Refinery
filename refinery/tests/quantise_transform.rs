//! Quantisation as a composable derived-corpus transform.
//!
//! Every test drives the public [`neat_ai_refinery::quantise`] API against a
//! real corpus on disk and asserts on the published result, so the checks
//! survive a change of implementation.
//!
//! Determinism is not a property a seed buys here: quantisation takes no seed,
//! so the same source must always produce the same bytes.

// The shared helpers serve several test binaries; not every one uses all of them.
#[allow(dead_code)]
mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use common::{encode, TempDir};
use neat_ai_refinery::corpus::{RecordShape, SourceCorpus, ValueEncoding};
use neat_ai_refinery::manifest::{CallerMetadata, Manifest, MANIFEST_FILE_NAME};
use neat_ai_refinery::quantise::{quantise, QuantiseError, QuantiseRequest, QuantiseScheme};
use neat_ai_refinery::sample::{sample, SampleRate, SampleRequest};
use neat_ai_refinery::transform::TransformError;

/// Two inputs and one output — twelve bytes a record as `f32`, six as bfloat16.
fn shape() -> RecordShape {
    RecordShape::new(2, 1).expect("valid shape")
}

/// The bound the bfloat16 scheme promises for `|q(x) - x| / |x|`.
const MAX_RELATIVE_ERROR: f32 = 1.0 / 256.0;

/// Values chosen to exercise the mapping rather than to round-trip cleanly:
/// exact powers of two, awkward mantissas, tiny and huge magnitudes, both
/// signs and a zero.
fn awkward_values(index: u32) -> [f32; 3] {
    let step = index as f32;
    [
        0.1 + step * 0.037,
        -(1.0e-12 * (step + 1.0)) * 3.7,
        (step - 8.0) * 1.0e9 + 0.5,
    ]
}

/// Writes `count` records of awkward values into `dir/name`, returning them.
fn write_shard(dir: &Path, name: &str, first: u32, count: u32) -> Vec<[f32; 3]> {
    let values: Vec<[f32; 3]> = (first..first + count).map(awkward_values).collect();
    let bytes: Vec<u8> = values.iter().flat_map(|record| encode(record)).collect();
    fs::write(dir.join(name), bytes).expect("write shard");
    values
}

/// A source directory holding one shard of `count` records.
fn source_with(root: &Path, count: u32) -> (PathBuf, Vec<[f32; 3]>) {
    let source = root.join("trainData-binary");
    fs::create_dir_all(&source).expect("create the source directory");
    let values = write_shard(&source, "shard-a.bin", 0, count);
    (source, values)
}

fn request(source: &Path, output: &Path) -> QuantiseRequest {
    QuantiseRequest {
        source: source.to_path_buf(),
        output: output.to_path_buf(),
        shape: shape(),
        scheme: QuantiseScheme::BFloat16,
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

/// Reads a published bfloat16 corpus back into decoded records.
fn read_quantised(path: &Path, records: u64) -> Vec<Vec<f32>> {
    let narrow = RecordShape::with_encoding(2, 1, ValueEncoding::BFloat16).expect("valid shape");
    let corpus = SourceCorpus::open(path, narrow).expect("open the published corpus");
    assert_eq!(corpus.record_count(), records);

    (0..records)
        .map(|index| corpus.read_record(index).expect("read the record"))
        .collect()
}

#[test]
fn publishes_every_record_in_order_under_the_scheme_name() {
    let temp = TempDir::new("quantise-publish");
    let (source, values) = source_with(temp.path(), 24);
    let output = temp.path().join("derived");

    let outcome = quantise(&request(&source, &output)).expect("the run succeeds");

    assert_eq!(outcome.records_read, 24);
    assert_eq!(
        outcome.records_written, outcome.records_read,
        "quantisation re-encodes records, it never drops them"
    );
    assert_eq!(outcome.output_file, output.join("quantise-bfloat16.bin"));
    assert_eq!(
        entries(&output),
        BTreeSet::from(["quantise-bfloat16.bin".into(), MANIFEST_FILE_NAME.into()]),
        "the corpus and its provenance are published together"
    );

    // Order is preserved exactly: record n of the output stands for record n of
    // the source, so a downstream index still means what it meant.
    let published = read_quantised(&outcome.output_file, 24);
    for (index, (original, decoded)) in values.iter().zip(&published).enumerate() {
        for (value, back) in original.iter().zip(decoded) {
            assert!(
                (back - value).abs() <= value.abs() * MAX_RELATIVE_ERROR,
                "record {index}: {value} came back as {back}"
            );
        }
    }
}

#[test]
fn halves_the_stored_bytes_of_the_corpus() {
    let temp = TempDir::new("quantise-storage");
    let (source, _) = source_with(temp.path(), 100);
    let output = temp.path().join("derived");

    let outcome = quantise(&request(&source, &output)).expect("the run succeeds");

    assert_eq!(outcome.source_bytes, 100 * 12);
    assert_eq!(outcome.output_bytes, 100 * 6);
    assert!(
        (outcome.storage_reduction() - 0.5).abs() < 1.0e-9,
        "expected a 50% reduction, got {}",
        outcome.storage_reduction()
    );
}

#[test]
fn holds_the_documented_relative_error_bound_across_the_corpus() {
    let temp = TempDir::new("quantise-error-bound");
    let (source, values) = source_with(temp.path(), 512);
    let output = temp.path().join("derived");

    let outcome = quantise(&request(&source, &output)).expect("the run succeeds");
    let published = read_quantised(&outcome.output_file, 512);

    let mut worst = 0.0_f32;
    for (original, decoded) in values.iter().zip(&published) {
        for (value, back) in original.iter().zip(decoded) {
            // A zero must survive exactly; there is no relative error to take.
            if *value == 0.0 {
                assert_eq!(*back, 0.0);
                continue;
            }
            let relative = (back - value).abs() / value.abs();
            assert!(
                relative <= MAX_RELATIVE_ERROR,
                "{value} lost {relative}, above the {MAX_RELATIVE_ERROR} bound"
            );
            worst = worst.max(relative);
        }
    }

    assert!(
        worst > 0.0,
        "the fixture must actually exercise the lossy path"
    );
    assert_eq!(
        f64::from(MAX_RELATIVE_ERROR),
        QuantiseScheme::BFloat16.max_relative_error(),
        "the bound asserted here is the one the manifest publishes"
    );
}

#[test]
fn is_deterministic_without_a_seed() {
    let temp = TempDir::new("quantise-deterministic");
    let (source, _) = source_with(temp.path(), 64);

    let first = quantise(&request(&source, &temp.path().join("first"))).expect("the first run");
    let second = quantise(&request(&source, &temp.path().join("second"))).expect("the second run");

    assert_eq!(
        fs::read(&first.output_file).expect("read the first corpus"),
        fs::read(&second.output_file).expect("read the second corpus"),
        "the same source must quantise to the same bytes"
    );
    assert_eq!(
        first.manifest.output.checksum.value, second.manifest.output.checksum.value,
        "and the checksums must agree"
    );
    assert_eq!(
        first.manifest.transform.seed, None,
        "quantisation takes no seed"
    );
}

#[test]
fn round_trips_a_value_the_scheme_represents_exactly() {
    let temp = TempDir::new("quantise-round-trip");
    let source = temp.path().join("trainData-binary");
    fs::create_dir_all(&source).expect("create the source directory");
    // Every one of these is exactly representable in eight significand bits.
    let exact: [[f32; 3]; 3] = [[1.0, -2.0, 0.5], [0.0, 256.0, -0.015625], [1.5, -3.5, 96.0]];
    let bytes: Vec<u8> = exact.iter().flat_map(|record| encode(record)).collect();
    fs::write(source.join("shard-a.bin"), bytes).expect("write the shard");
    let output = temp.path().join("derived");

    let outcome = quantise(&request(&source, &output)).expect("the run succeeds");
    let published = read_quantised(&outcome.output_file, 3);

    for (original, decoded) in exact.iter().zip(&published) {
        assert_eq!(
            decoded.as_slice(),
            original.as_slice(),
            "an exactly representable record must survive the round trip unchanged"
        );
    }
}

#[test]
fn records_the_scheme_and_both_layouts_in_the_manifest() {
    let temp = TempDir::new("quantise-manifest");
    let (source, _) = source_with(temp.path(), 8);
    let output = temp.path().join("derived");
    let mut request = request(&source, &output);
    request.metadata =
        CallerMetadata::parse(&["grq_observation_version=42".to_string()]).expect("valid metadata");

    let outcome = quantise(&request).expect("the run succeeds");
    let manifest = Manifest::load(&outcome.manifest_file).expect("read the published manifest");

    assert_eq!(manifest.transform.name, "quantise");
    assert_eq!(manifest.transform.seed, None);
    // Explicit parameters: the mapping is stated, never left to be inferred.
    assert_eq!(manifest.transform.parameters["scheme"], "bfloat16");
    assert_eq!(manifest.transform.parameters["source_encoding"], "float32");
    assert_eq!(manifest.transform.parameters["target_encoding"], "bfloat16");
    assert_eq!(
        manifest.transform.parameters["rounding"],
        "nearest-ties-to-even"
    );
    assert_eq!(
        manifest.transform.parameters["max_relative_error"],
        1.0 / 256.0
    );

    // `record_shape` describes what was published; the source layout is stated
    // separately because this transform changed it.
    assert_eq!(manifest.record_shape.encoding, "bfloat16");
    assert_eq!(manifest.record_shape.bytes_per_record, 6);
    assert_eq!(manifest.record_shape.record_values, 3);
    let source_shape = manifest
        .source_record_shape
        .as_ref()
        .expect("a representation transform records the layout it read");
    assert_eq!(source_shape.encoding, "float32");
    assert_eq!(source_shape.bytes_per_record, 12);

    assert_eq!(manifest.source.record_count, 8);
    assert_eq!(manifest.output.record_count, 8);
    assert_eq!(manifest.output.bytes, 48);
    assert_eq!(manifest.output.file, "quantise-bfloat16.bin");
    assert_eq!(manifest.metadata.get("grq_observation_version"), Some("42"));
    assert_eq!(
        manifest.output.checksum.value,
        outcome.manifest.output.checksum.value
    );
}

#[test]
fn composes_with_sampling_over_the_published_corpus() {
    let temp = TempDir::new("quantise-composes");
    let (source, _) = source_with(temp.path(), 200);
    let sampled = temp.path().join("sampled");
    let quantised = temp.path().join("sampled-bf16");

    // Sample first — an ordinary run, with no knowledge that anything follows.
    let sample_outcome = sample(&SampleRequest {
        source: source.clone(),
        output: sampled.clone(),
        shape: shape(),
        rate: SampleRate::new(0.25).expect("valid rate"),
        seed: Some(20_260_831),
        metadata: CallerMetadata::default(),
    })
    .expect("the sampling run succeeds");

    // Then quantise its output — an ordinary run over an ordinary corpus
    // directory, with no sampler-specific or application-specific handling.
    let outcome = quantise(&request(&sampled, &quantised)).expect("the quantisation run succeeds");

    assert_eq!(
        outcome.records_read, sample_outcome.records_written,
        "the second transform reads exactly what the first published"
    );
    assert_eq!(outcome.records_written, outcome.records_read);
    assert_eq!(outcome.output_bytes * 2, outcome.source_bytes);
    assert_eq!(
        outcome.sources,
        vec![sampled.join(sample_outcome.manifest.output.file.clone())],
        "the manifest beside the corpus is not mistaken for records"
    );

    // The sampled corpus is a source now, and sources are immutable.
    let manifest = Manifest::load(&outcome.manifest_file).expect("read the manifest");
    assert_eq!(manifest.source.file_count, 1);
    assert_eq!(manifest.source.record_count, sample_outcome.records_written);
    assert!(
        sampled.join("sample-25.bin").exists(),
        "the sampled corpus survives being quantised"
    );
}

#[test]
fn refuses_to_quantise_a_corpus_that_is_already_quantised() {
    let temp = TempDir::new("quantise-twice");
    let (source, _) = source_with(temp.path(), 16);
    let once = temp.path().join("once");
    let twice = temp.path().join("twice");

    quantise(&request(&source, &once)).expect("the first run succeeds");

    // The published manifest declares bfloat16; reading those bytes as `f32`
    // would silently reinterpret them, so the run must fail loud instead.
    let error = quantise(&request(&once, &twice)).expect_err("a second pass is refused");

    assert!(
        matches!(error, QuantiseError::SourceEncodingMismatch { ref found, .. } if found == "bfloat16"),
        "{error:?}"
    );
    assert!(!twice.exists(), "nothing is published");
}

#[test]
fn refuses_a_record_shape_the_source_manifest_contradicts() {
    let temp = TempDir::new("quantise-wrong-shape");
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
    // Five values a record, not three: the records would be split in the wrong
    // places, and every value after the first would be nonsense.
    request.shape = RecordShape::new(4, 1).expect("valid shape");

    let error = quantise(&request).expect_err("the declared width is checked");

    assert!(
        matches!(
            error,
            QuantiseError::SourceWidthMismatch {
                expected: 20,
                found: 12,
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn fails_loud_on_a_source_manifest_it_cannot_read() {
    let temp = TempDir::new("quantise-broken-manifest");
    let (source, _) = source_with(temp.path(), 4);
    fs::write(source.join(MANIFEST_FILE_NAME), b"{ not a manifest").expect("write the fixture");
    let output = temp.path().join("derived");

    let error = quantise(&request(&source, &output)).expect_err("a broken manifest is fatal");

    assert!(
        matches!(error, QuantiseError::Transform(TransformError::Manifest(_))),
        "{error:?}"
    );
    assert!(!output.exists(), "nothing is published");
}

#[test]
fn reads_a_raw_corpus_that_carries_no_manifest() {
    let temp = TempDir::new("quantise-no-manifest");
    let (source, _) = source_with(temp.path(), 4);
    assert!(!source.join(MANIFEST_FILE_NAME).exists());

    let outcome = quantise(&request(&source, &temp.path().join("derived")))
        .expect("a raw source corpus is read as the caller described it");

    assert_eq!(outcome.records_written, 4);
}

#[test]
fn leaves_the_source_corpus_byte_for_byte_unchanged() {
    let temp = TempDir::new("quantise-immutable-source");
    let (source, _) = source_with(temp.path(), 32);
    let shard = source.join("shard-a.bin");
    let before = fs::read(&shard).expect("read the source");

    quantise(&request(&source, &temp.path().join("derived"))).expect("the run succeeds");

    assert_eq!(
        fs::read(&shard).expect("read the source again"),
        before,
        "the source corpus is never written to"
    );
    assert_eq!(entries(&source), BTreeSet::from(["shard-a.bin".into()]));
}

#[test]
fn refuses_an_output_directory_that_overlaps_the_source() {
    let temp = TempDir::new("quantise-overlap");
    let (source, _) = source_with(temp.path(), 4);

    let error = quantise(&request(&source, &source.join("inside")))
        .expect_err("publishing inside the source would delete it");

    assert!(
        matches!(
            error,
            QuantiseError::Transform(TransformError::OverlappingCorpora { .. })
        ),
        "{error:?}"
    );
}

#[test]
fn refuses_a_source_directory_holding_no_corpus_files() {
    let temp = TempDir::new("quantise-empty-source");
    let source = temp.path().join("trainData-binary");
    fs::create_dir_all(&source).expect("create the source directory");
    fs::write(source.join("notes.txt"), b"not a corpus").expect("write the note");

    let error = quantise(&request(&source, &temp.path().join("derived")))
        .expect_err("there is nothing to quantise");

    assert!(
        matches!(
            error,
            QuantiseError::Transform(TransformError::NoCorpusFiles { .. })
        ),
        "{error:?}"
    );
}

#[test]
fn fails_loud_on_a_partial_trailing_record() {
    let temp = TempDir::new("quantise-partial-record");
    let source = temp.path().join("trainData-binary");
    fs::create_dir_all(&source).expect("create the source directory");
    let mut bytes = encode(&[1.0, 2.0, 3.0]);
    bytes.extend_from_slice(&[0_u8; 5]);
    fs::write(source.join("shard-a.bin"), bytes).expect("write the shard");
    let output = temp.path().join("derived");

    let error = quantise(&request(&source, &output)).expect_err("a partial record is fatal");

    assert!(
        matches!(error, QuantiseError::Transform(TransformError::Corpus(_))),
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
    let temp = TempDir::new("quantise-republish");
    let (source, _) = source_with(temp.path(), 8);
    let output = temp.path().join("derived");
    fs::create_dir_all(&output).expect("create the live directory");
    fs::write(output.join("stale.bin"), b"stale").expect("write the stale corpus");

    quantise(&request(&source, &output)).expect("the run succeeds");

    assert_eq!(
        entries(&output),
        BTreeSet::from(["quantise-bfloat16.bin".into(), MANIFEST_FILE_NAME.into()]),
        "the live directory is replaced whole, not merged into"
    );
}
