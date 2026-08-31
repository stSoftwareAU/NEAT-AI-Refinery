//! The production-soak evidence harness (issue #8).
//!
//! Cutting GRQ over to Refinery is gated on evidence, not on confidence, so
//! the evidence is produced by code that is itself tested: a soak run measures
//! both samplers the same way, re-verifies the published corpus geometry,
//! proves the source corpus was not touched, and proves a failed run leaves
//! the previously published corpus exactly as it was.
//!
//! Every invariant below is asserted by calling the real API — nothing here
//! inspects source text.
//!
//! The Deno comparison needs `deno` on `PATH`. Without it the one test that
//! uses it prints a skip notice and passes, matching `parity_harness.rs`.

// The shared helpers serve several test binaries; not every one uses all of them.
#[allow(dead_code)]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{encode, TempDir};
use neat_ai_refinery::corpus::RecordShape;
use neat_ai_refinery::manifest::{CallerMetadata, Manifest};
use neat_ai_refinery::sample::{sample, SampleRate, SampleRequest};
use neat_ai_refinery::soak::{
    soak, CorpusDigest, DenoReference, MeasuredCommand, PublishedCorpus, SoakConfig, SoakError,
};

/// The sampler binary under test, built by `cargo test`.
const BINARY: &str = env!("CARGO_BIN_EXE_neat_ai_refinery");

/// Two inputs and one output — twelve bytes a record, as the other suites use.
const INPUTS: usize = 2;
const OUTPUTS: usize = 1;

fn shape() -> RecordShape {
    RecordShape::new(INPUTS, OUTPUTS).expect("valid shape")
}

/// The `parity/` directory holding the Deno reference sampler.
fn parity_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crate sits inside the workspace")
        .join("parity")
}

/// Is `deno` runnable? A missing `deno` skips, exactly as the parity harness.
fn deno_available(test: &str) -> bool {
    let found = Command::new("deno")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !found {
        eprintln!("SKIPPED {test}: `deno` is not on PATH — install Deno to compare the samplers");
    }
    found
}

/// Writes a small source corpus of `records` whole records into `directory`.
fn write_corpus(directory: &Path, records: usize) {
    fs::create_dir_all(directory).expect("create the source directory");
    let mut bytes = Vec::new();
    for record in 0..records {
        let value = record as f32;
        bytes.extend_from_slice(&encode(&[value, value + 0.5, value + 1.0]));
    }
    fs::write(directory.join("shard-a.bin"), &bytes).expect("write the corpus");
}

/// Publishes a real derived corpus and returns the directory it was published
/// to, so the verification tests work against genuine output.
fn publish_corpus(root: &Path) -> PathBuf {
    let source = root.join("trainData-binary");
    let output = root.join("trainData-binary-sampler");
    write_corpus(&source, 40);

    let request = SampleRequest {
        source,
        output: output.clone(),
        shape: shape(),
        rate: SampleRate::new(1.0).expect("a valid rate"),
        seed: Some(20_260_831),
        metadata: CallerMetadata::default(),
    };
    sample(&request).expect("publish the derived corpus");
    output
}

#[test]
fn measures_the_wall_clock_and_peak_memory_of_a_live_process() {
    let temp = TempDir::new("soak-measure");

    let measurement = MeasuredCommand::new("sleep", "sleep", temp.path())
        .arg("1")
        .measure()
        .expect("measure a process that succeeds");

    assert!(
        measurement.elapsed_ms >= 900,
        "a one second process was timed at {}ms",
        measurement.elapsed_ms
    );
    let peak = measurement
        .peak_rss_kib
        .expect("a process alive for a second is sampled at least once");
    assert!(peak > 0, "peak RSS was sampled as {peak} KiB");
    assert_eq!(measurement.label, "sleep");
}

#[test]
fn a_failed_command_fails_loud_with_what_it_wrote() {
    let temp = TempDir::new("soak-measure-failure");

    let error = MeasuredCommand::new("refinery", BINARY, temp.path())
        .args(["--source", "/does/not/exist"])
        .args(["--output", "/tmp/never-published"])
        .args(["--inputs", "2", "--outputs", "1"])
        .args(["sample", "--rate", "0.5"])
        .measure()
        .expect_err("a sampler that cannot read its source must fail loud");

    match error {
        SoakError::CommandFailed {
            label,
            code,
            stderr,
        } => {
            assert_eq!(label, "refinery");
            assert_ne!(code, Some(0), "a failure reported a success code");
            assert!(!stderr.is_empty(), "the failure captured no diagnostics");
        }
        other => panic!("expected a command failure, got {other:?}"),
    }
}

#[test]
fn a_missing_program_fails_loud_rather_than_reporting_zero() {
    let temp = TempDir::new("soak-measure-missing");

    let error = MeasuredCommand::new("absent", "/does/not/exist/neat_ai_refinery", temp.path())
        .measure()
        .expect_err("a binary that cannot be spawned is not a zero-cost run");

    assert!(matches!(error, SoakError::Spawn { .. }), "{error:?}");
}

#[test]
fn digests_notice_a_source_corpus_that_changed() {
    let temp = TempDir::new("soak-digest");
    let source = temp.path().join("trainData-binary");
    write_corpus(&source, 10);

    let before = CorpusDigest::of(&source).expect("digest the source");
    let unchanged = CorpusDigest::of(&source).expect("digest the source again");
    assert_eq!(
        before, unchanged,
        "an untouched corpus digested differently"
    );
    assert_eq!(before.files.len(), 1);
    assert_eq!(before.files[0].name, "shard-a.bin");
    assert!(before.files[0].bytes > 0);

    write_corpus(&source, 11);
    let after = CorpusDigest::of(&source).expect("digest the mutated source");

    assert_ne!(before, after, "a mutated corpus digested identically");
}

#[test]
fn verifies_a_genuinely_published_corpus() {
    let temp = TempDir::new("soak-verify");
    let published = publish_corpus(temp.path());

    let corpus = PublishedCorpus::verify(&published, shape()).expect("verify the published corpus");

    assert_eq!(corpus.file, "sample-100.bin");
    assert_eq!(corpus.record_count, 40, "a rate of 1 keeps every record");
    assert_eq!(corpus.records_read, 40);
    assert_eq!(corpus.bytes, 40 * shape().bytes_per_record() as u64);
}

#[test]
fn rejects_a_published_corpus_holding_a_partial_record() {
    let temp = TempDir::new("soak-verify-partial");
    let published = publish_corpus(temp.path());
    let corpus_file = published.join("sample-100.bin");
    let mut bytes = fs::read(&corpus_file).expect("read the published corpus");
    bytes.truncate(bytes.len() - 1);
    fs::write(&corpus_file, &bytes).expect("truncate the published corpus");

    let error = PublishedCorpus::verify(&published, shape())
        .expect_err("a partial trailing record is not valid geometry");

    assert!(matches!(error, SoakError::Invariant { .. }), "{error:?}");
}

#[test]
fn rejects_a_published_corpus_whose_bytes_no_longer_match_the_manifest() {
    let temp = TempDir::new("soak-verify-tampered");
    let published = publish_corpus(temp.path());
    let corpus_file = published.join("sample-100.bin");
    let mut bytes = fs::read(&corpus_file).expect("read the published corpus");
    bytes[0] ^= 0xff;
    fs::write(&corpus_file, &bytes).expect("tamper with the published corpus");

    let error = PublishedCorpus::verify(&published, shape())
        .expect_err("a corpus that no longer matches its checksum is not evidence");

    assert!(matches!(error, SoakError::Invariant { .. }), "{error:?}");
}

#[test]
fn rejects_a_published_corpus_the_manifest_miscounts() {
    let temp = TempDir::new("soak-verify-miscount");
    let published = publish_corpus(temp.path());
    let manifest_file = published.join("manifest.json");
    let mut manifest = Manifest::load(&manifest_file).expect("read the manifest");
    manifest.output.record_count += 1;
    fs::write(
        &manifest_file,
        serde_json::to_vec_pretty(&manifest).expect("encode the manifest"),
    )
    .expect("write the manifest back");

    let error = PublishedCorpus::verify(&published, shape())
        .expect_err("a manifest count that disagrees with the bytes is fatal");

    assert!(matches!(error, SoakError::Invariant { .. }), "{error:?}");
}

#[test]
fn rejects_a_published_corpus_of_another_record_shape() {
    let temp = TempDir::new("soak-verify-shape");
    let published = publish_corpus(temp.path());

    let error = PublishedCorpus::verify(&published, RecordShape::new(5, 1).expect("valid shape"))
        .expect_err("a corpus of another geometry is not the one that was asked for");

    assert!(matches!(error, SoakError::Invariant { .. }), "{error:?}");
}

/// The soak configuration the tests use — small enough to run in seconds.
fn config(workspace: &Path, reference: Option<DenoReference>) -> SoakConfig {
    SoakConfig {
        workspace: workspace.to_path_buf(),
        binary: PathBuf::from(BINARY),
        shape: shape(),
        shards: 2,
        records_per_shard: 400,
        rate: SampleRate::new(0.5).expect("a valid rate"),
        rounds: 2,
        reference,
    }
}

#[test]
fn a_soak_run_captures_the_evidence_the_cut_over_needs() {
    let temp = TempDir::new("soak-run");

    let report = soak(&config(temp.path(), None)).expect("run the soak");

    assert_eq!(report.rounds.len(), 2, "both rounds should be reported");
    for round in &report.rounds {
        let published = &round.published;
        assert_eq!(published.records_read, 800, "the whole corpus is read");
        assert!(
            (200..=600).contains(&published.record_count),
            "a rate of 0.5 kept {} of 800 records",
            published.record_count
        );
        assert!(round.measurement.elapsed_ms > 0);
    }

    assert!(
        report.source_unchanged,
        "the soak must prove the source corpus was not written to"
    );
    assert!(
        report.atomic_publication.previous_corpus_intact,
        "a failed run must leave the published corpus exactly as it was"
    );
    assert_eq!(
        report.atomic_publication.scratch_left_behind, 0,
        "a failed run left staging directories behind"
    );
    assert!(
        report.reference.is_none(),
        "no reference sampler was asked for"
    );
    assert!(!report.host.os.is_empty(), "the host must be identified");
}

#[test]
fn a_soak_report_renders_as_committable_evidence() {
    let temp = TempDir::new("soak-report");
    let report = soak(&config(temp.path(), None)).expect("run the soak");

    let json = report.to_json().expect("encode the report");
    let markdown = report.to_markdown();

    let parsed: serde_json::Value = serde_json::from_str(&json).expect("the report is JSON");
    assert_eq!(parsed["host"]["os"], serde_json::json!(report.host.os));
    assert_eq!(parsed["rounds"].as_array().expect("rounds").len(), 2);
    assert_eq!(parsed["source_unchanged"], serde_json::json!(true));
    assert!(markdown.contains("| round |"), "{markdown}");
    assert!(markdown.contains(&report.host.arch), "{markdown}");
    assert!(
        markdown.contains("no source corpus mutation"),
        "the evidence must state the immutability result: {markdown}"
    );
}

#[test]
fn a_soak_run_compares_refinery_against_the_deno_sampler() {
    if !deno_available("a_soak_run_compares_refinery_against_the_deno_sampler") {
        return;
    }
    let temp = TempDir::new("soak-reference");
    let reference = DenoReference {
        parity_dir: parity_dir(),
        check_consumer: false,
    };

    let report = soak(&config(temp.path(), Some(reference))).expect("run the soak");

    let deno = report
        .reference
        .as_ref()
        .expect("the Deno sampler was measured");
    assert!(deno.elapsed_ms > 0, "the Deno run was not timed");
    assert_eq!(deno.label, "typescript");
    assert!(
        report.reference_records_written.is_some(),
        "the Deno run must report what it published so the counts can be compared"
    );
    assert!(report.consumer.is_none(), "no consumer check was asked for");
}
