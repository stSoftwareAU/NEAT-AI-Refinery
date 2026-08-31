//! The published-artefact contract an orchestrator integrates against.
//!
//! GRQ runs Refinery behind a rollback switch (issue #7) and reads the run's
//! record counts out of `manifest.json` rather than by parsing console output,
//! so those JSON key names are a public interface: renaming one would leave
//! GRQ unable to measure a run it just published, with nothing in this crate
//! failing. The assertions below are therefore over the **raw JSON**, not the
//! Rust structs — a serde rename that keeps the Rust field intact still breaks
//! a consumer, and must break this test.
//!
//! See `docs/grq-integration.md` for the caller's side of the contract.

// The shared helpers serve several test binaries; not every one uses all of them.
#[allow(dead_code)]
mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use common::{encode, TempDir};
use neat_ai_refinery::corpus::RecordShape;
use neat_ai_refinery::manifest::CallerMetadata;
use neat_ai_refinery::sample::{sample, SampleRate, SampleRequest};
use serde_json::Value;

/// Writes `count` records of the unit-test shape into `dir/name`.
fn write_shard(dir: &Path, name: &str, first: u32, count: u32) {
    let bytes: Vec<u8> = (first..first + count)
        .flat_map(|index| {
            let value = index as f32;
            encode(&[value, value * 2.0, value * 3.0])
        })
        .collect();
    fs::write(dir.join(name), bytes).expect("write shard");
}

/// Publishes a corpus and returns the parsed manifest and the output path.
fn publish(label: &str, rate: f64, records: u32) -> (Value, TempDir) {
    let temp = TempDir::new(label);
    let source = temp.path().join("trainData-binary");
    fs::create_dir_all(&source).expect("create the source directory");
    write_shard(&source, "shard-a.bin", 0, records);
    let output = temp.path().join("trainData-binary-sampler");

    sample(&SampleRequest {
        source: source.clone(),
        output: output.clone(),
        shape: RecordShape::new(2, 1).expect("valid shape"),
        rate: SampleRate::new(rate).expect("valid rate"),
        seed: Some(20_260_831),
        metadata: CallerMetadata::parse(&["grq_observation_version=42".to_string()])
            .expect("valid metadata"),
    })
    .expect("sampling");

    let text = fs::read_to_string(output.join("manifest.json")).expect("read the manifest");
    (
        serde_json::from_str(&text).expect("the manifest is JSON"),
        temp,
    )
}

/// The counts a caller compares an old and a new sampler run with.
#[test]
fn the_manifest_names_the_counts_an_orchestrator_reads() {
    let (manifest, _temp) = publish("consumer-counts", 1.0, 100);

    assert_eq!(
        manifest.pointer("/output/file").and_then(Value::as_str),
        Some("sample-100.bin"),
        "`output.file` names the published corpus: {manifest}"
    );
    assert_eq!(
        manifest
            .pointer("/output/record_count")
            .and_then(Value::as_u64),
        Some(100),
        "`output.record_count` is how many records were kept: {manifest}"
    );
    assert_eq!(
        manifest
            .pointer("/source/record_count")
            .and_then(Value::as_u64),
        Some(100),
        "`source.record_count` is how many records were read: {manifest}"
    );
    assert_eq!(
        manifest
            .pointer("/metadata/grq_observation_version")
            .and_then(Value::as_str),
        Some("42"),
        "caller metadata is stored verbatim under its own key: {manifest}"
    );
}

/// The counts must describe the sample, not the source it was drawn from.
#[test]
fn the_manifest_counts_distinguish_records_read_from_records_kept() {
    let (manifest, _temp) = publish("consumer-sample", 0.5, 400);

    let read = manifest
        .pointer("/source/record_count")
        .and_then(Value::as_u64)
        .expect("records read");
    let written = manifest
        .pointer("/output/record_count")
        .and_then(Value::as_u64)
        .expect("records written");

    assert_eq!(read, 400, "every source record was read: {manifest}");
    assert!(
        written < read && written > 0,
        "a rate of 0.5 keeps some but not all of {read} records, kept {written}"
    );
}

/// A consumer resolves the corpus by scanning for `.bin`, so the published
/// directory must hold exactly the corpus and its provenance.
#[test]
fn the_published_directory_holds_only_the_corpus_and_its_manifest() {
    let temp = TempDir::new("consumer-directory");
    let source = temp.path().join("trainData-binary");
    fs::create_dir_all(&source).expect("create the source directory");
    write_shard(&source, "shard-a.bin", 0, 40);
    let output = temp.path().join("trainData-binary-sampler");

    sample(&SampleRequest {
        source,
        output: output.clone(),
        shape: RecordShape::new(2, 1).expect("valid shape"),
        rate: SampleRate::new(1.0).expect("valid rate"),
        seed: None,
        metadata: CallerMetadata::default(),
    })
    .expect("sampling");

    let entries: BTreeSet<String> = fs::read_dir(&output)
        .expect("read the published directory")
        .map(|entry| entry.expect("entry").file_name().to_string_lossy().into())
        .collect();

    assert_eq!(
        entries,
        BTreeSet::from(["manifest.json".to_string(), "sample-100.bin".to_string()]),
        "exactly one corpus file and one manifest are published"
    );
}
