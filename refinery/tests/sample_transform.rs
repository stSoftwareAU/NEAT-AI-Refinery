//! The materialised sampler, ported from GRQ's `src/train/Sampler.ts`.
//!
//! Every test drives the public [`neat_ai_refinery::sample`] API against a real
//! corpus on disk and asserts on the published result, so the checks survive a
//! change of implementation.

// The shared helpers serve several test binaries; not every one uses all of them.
#[allow(dead_code)]
mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use common::{encode, TempDir};
use neat_ai_refinery::corpus::RecordShape;
use neat_ai_refinery::sample::{sample, SampleError, SampleRate, SampleRequest};

/// Two inputs and one output — twelve bytes a record.
fn shape() -> RecordShape {
    RecordShape::new(2, 1).expect("valid shape")
}

/// A record whose values identify it uniquely.
fn record(index: u32) -> Vec<u8> {
    let value = index as f32;
    encode(&[value, value * 2.0, value * 3.0])
}

/// Writes `count` records starting at `first` into `dir/name`.
fn write_shard(dir: &Path, name: &str, first: u32, count: u32) -> Vec<Vec<u8>> {
    let records: Vec<Vec<u8>> = (first..first + count).map(record).collect();
    let bytes: Vec<u8> = records.iter().flatten().copied().collect();
    fs::write(dir.join(name), bytes).expect("write shard");
    records
}

/// Splits a published sample back into whole records.
fn read_records(path: &Path) -> Vec<Vec<u8>> {
    let bytes = fs::read(path).expect("read the published sample");
    assert_eq!(bytes.len() % 12, 0, "the sample must hold whole records");
    bytes.chunks(12).map(<[u8]>::to_vec).collect()
}

/// Every file name in `dir`, sorted.
fn entries(dir: &Path) -> BTreeSet<String> {
    fs::read_dir(dir)
        .expect("read the directory")
        .map(|entry| entry.expect("read the entry").file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect()
}

/// A source directory holding one shard of `count` records.
fn source_with(root: &Path, count: u32) -> (PathBuf, Vec<Vec<u8>>) {
    let source = root.join("trainData-binary");
    fs::create_dir_all(&source).expect("create the source directory");
    let records = write_shard(&source, "shard-a.bin", 0, count);
    (source, records)
}

fn request(source: &Path, output: &Path, rate: f64, seed: Option<u64>) -> SampleRequest {
    SampleRequest {
        source: source.to_path_buf(),
        output: output.to_path_buf(),
        shape: shape(),
        rate: SampleRate::new(rate).expect("valid rate"),
        seed,
    }
}

#[test]
fn rejects_a_rate_outside_the_allowed_range() {
    for rate in [0.0, -0.1, 1.000_001, 2.0, f64::NAN, f64::INFINITY] {
        let error = SampleRate::new(rate).expect_err("the rate must be rejected");
        assert!(
            matches!(error, SampleError::InvalidRate { .. }),
            "{rate} — {error:?}"
        );
    }
}

#[test]
fn accepts_the_range_the_deno_sampler_accepts() {
    for rate in [f64::MIN_POSITIVE, 0.001, 0.05, 0.5, 1.0] {
        let parsed = SampleRate::new(rate).expect("the rate must be accepted");
        assert_eq!(parsed.value(), rate);
    }
}

#[test]
fn names_the_output_by_the_rounded_percentage() {
    let cases = [
        (1.0, "sample-100.bin"),
        (0.3, "sample-30.bin"),
        (0.05, "sample-5.bin"),
        (0.125, "sample-13.bin"),
        (0.001, "sample-0.bin"),
    ];

    for (rate, expected) in cases {
        let rate = SampleRate::new(rate).expect("valid rate");
        assert_eq!(rate.file_name(), expected);
    }
}

#[test]
fn publishes_every_record_at_a_rate_of_one() {
    let temp = TempDir::new("sample-full");
    let (source, records) = source_with(temp.path(), 64);
    let output = temp.path().join("trainData-binary-sampler");

    let outcome = sample(&request(&source, &output, 1.0, Some(7))).expect("sampling succeeds");

    assert_eq!(outcome.records_read, 64);
    assert_eq!(outcome.records_written, 64);
    assert_eq!(entries(&output), BTreeSet::from(["sample-100.bin".into()]));

    let published = read_records(&output.join("sample-100.bin"));
    assert_eq!(
        published.iter().collect::<BTreeSet<_>>(),
        records.iter().collect::<BTreeSet<_>>(),
        "a full-rate sample is a permutation of the source"
    );
}

#[test]
fn selects_each_record_independently_at_the_sample_rate() {
    let temp = TempDir::new("sample-rate");
    let (source, _) = source_with(temp.path(), 4_000);
    let output = temp.path().join("trainData-binary-sampler");

    let outcome =
        sample(&request(&source, &output, 0.25, Some(20_260_831))).expect("sampling succeeds");

    assert_eq!(outcome.records_read, 4_000);
    assert!(
        (850..=1_150).contains(&outcome.records_written),
        "a quarter of 4000 records should land near 1000, got {}",
        outcome.records_written
    );
    assert_eq!(
        read_records(&output.join("sample-25.bin")).len() as u64,
        outcome.records_written
    );
}

#[test]
fn repeats_exactly_for_a_given_seed_and_differs_across_seeds() {
    let temp = TempDir::new("sample-seed");
    let (source, _) = source_with(temp.path(), 64);

    let first = temp.path().join("first");
    let again = temp.path().join("again");
    let other = temp.path().join("other");

    sample(&request(&source, &first, 1.0, Some(42))).expect("first run");
    sample(&request(&source, &again, 1.0, Some(42))).expect("repeat run");
    sample(&request(&source, &other, 1.0, Some(43))).expect("other seed");

    let sample_of = |dir: &Path| fs::read(dir.join("sample-100.bin")).expect("read the sample");
    assert_eq!(
        sample_of(&first),
        sample_of(&again),
        "the same seed must reproduce the sample byte for byte"
    );
    assert_ne!(
        sample_of(&first),
        sample_of(&other),
        "a different seed must reorder 64 records"
    );
}

#[test]
fn reports_the_seed_it_used_when_none_was_supplied() {
    let temp = TempDir::new("sample-unseeded");
    let (source, _) = source_with(temp.path(), 8);
    let output = temp.path().join("derived");

    let outcome = sample(&request(&source, &output, 1.0, None)).expect("sampling succeeds");

    let replay = temp.path().join("replay");
    let replayed =
        sample(&request(&source, &replay, 1.0, Some(outcome.seed))).expect("replay succeeds");

    assert_eq!(replayed.seed, outcome.seed);
    assert_eq!(
        fs::read(output.join("sample-100.bin")).expect("read the sample"),
        fs::read(replay.join("sample-100.bin")).expect("read the replay"),
        "the reported seed must reproduce the run"
    );
}

#[test]
fn leaves_the_source_corpus_untouched() {
    let temp = TempDir::new("sample-immutable");
    let source = temp.path().join("trainData-binary");
    fs::create_dir_all(&source).expect("create the source directory");
    write_shard(&source, "shard-a.bin", 0, 32);
    write_shard(&source, "shard-b.bin", 32, 32);
    let before: Vec<(String, Vec<u8>)> = snapshot(&source);

    sample(&request(
        &source,
        &temp.path().join("derived"),
        0.5,
        Some(11),
    ))
    .expect("sampling succeeds");

    assert_eq!(snapshot(&source), before, "the source must be unchanged");
}

/// Every source file name paired with its bytes.
fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = fs::read_dir(dir)
        .expect("read the source directory")
        .map(|entry| entry.expect("read the entry").path())
        .map(|path| {
            (
                path.file_name()
                    .expect("a file name")
                    .to_string_lossy()
                    .into_owned(),
                fs::read(&path).expect("read the source file"),
            )
        })
        .collect();
    files.sort();
    files
}

#[test]
fn reads_every_bin_shard_and_ignores_everything_else() {
    let temp = TempDir::new("sample-shards");
    let source = temp.path().join("trainData-binary");
    fs::create_dir_all(&source).expect("create the source directory");
    write_shard(&source, "shard-a.bin", 0, 10);
    write_shard(&source, "shard-b.bin", 10, 10);
    fs::write(source.join("notes.txt"), b"not a corpus").expect("write the stray file");
    fs::create_dir_all(source.join("nested")).expect("create a nested directory");

    let output = temp.path().join("derived");
    let outcome = sample(&request(&source, &output, 1.0, Some(3))).expect("sampling succeeds");

    assert_eq!(outcome.records_read, 20);
    assert_eq!(outcome.records_written, 20);
    assert_eq!(outcome.sources.len(), 2);
}

#[test]
fn republishes_over_a_live_derived_corpus() {
    let temp = TempDir::new("sample-republish");
    let (source, _) = source_with(temp.path(), 16);
    let output = temp.path().join("trainData-binary-sampler");
    fs::create_dir_all(&output).expect("create the live directory");
    fs::write(output.join("sample-99.bin"), b"stale").expect("write the stale sample");

    sample(&request(&source, &output, 1.0, Some(5))).expect("sampling succeeds");

    assert_eq!(
        entries(&output),
        BTreeSet::from(["sample-100.bin".into()]),
        "publishing replaces the whole directory rather than editing it in place"
    );
    assert!(
        !leftovers(temp.path())
            .iter()
            .any(|name| name.contains("deleting") || name.contains("staging")),
        "publishing leaves no scratch directories behind: {:?}",
        leftovers(temp.path())
    );
}

/// Names directly under `root`, used to prove no scratch survives.
fn leftovers(root: &Path) -> Vec<String> {
    entries(root).into_iter().collect()
}

#[test]
fn fails_loud_on_a_partial_record_and_removes_the_staging_directory() {
    let temp = TempDir::new("sample-partial");
    let source = temp.path().join("trainData-binary");
    fs::create_dir_all(&source).expect("create the source directory");
    write_shard(&source, "shard-a.bin", 0, 4);
    fs::write(source.join("shard-b.bin"), vec![0_u8; 13]).expect("write a truncated shard");

    let output = temp.path().join("derived");
    let error =
        sample(&request(&source, &output, 1.0, Some(9))).expect_err("a partial record is fatal");

    assert!(
        matches!(error, SampleError::Corpus(_)),
        "a malformed record surfaces as a corpus error: {error:?}"
    );
    assert!(!output.exists(), "a failed run publishes nothing");
    assert_eq!(
        leftovers(temp.path()),
        vec!["trainData-binary".to_string()],
        "the staging directory is removed when the run fails"
    );
}

#[test]
fn rejects_a_source_directory_holding_no_corpus_files() {
    let temp = TempDir::new("sample-empty-dir");
    let source = temp.path().join("trainData-binary");
    fs::create_dir_all(&source).expect("create the source directory");
    fs::write(source.join("readme.txt"), b"no corpus here").expect("write the stray file");

    let error = sample(&request(
        &source,
        &temp.path().join("derived"),
        1.0,
        Some(1),
    ))
    .expect_err("a source with no .bin files is fatal");

    assert!(
        matches!(error, SampleError::NoCorpusFiles { .. }),
        "{error:?}"
    );
}

#[test]
fn refuses_to_publish_onto_the_source_directory() {
    let temp = TempDir::new("sample-onto-source");
    let (source, _) = source_with(temp.path(), 4);

    let error = sample(&request(&source, &source, 1.0, Some(1)))
        .expect_err("publishing over the source would destroy it");

    assert!(
        matches!(error, SampleError::OutputInsideSource { .. }),
        "{error:?}"
    );
    assert_eq!(
        entries(&source),
        BTreeSet::from(["shard-a.bin".into()]),
        "the source survives the rejected run"
    );
}

#[test]
fn refuses_to_publish_inside_the_source_directory() {
    let temp = TempDir::new("sample-inside-source");
    let (source, _) = source_with(temp.path(), 4);

    let error = sample(&request(&source, &source.join("derived"), 1.0, Some(1)))
        .expect_err("a derived corpus inside the source is rejected");

    assert!(
        matches!(error, SampleError::OutputInsideSource { .. }),
        "{error:?}"
    );
}
