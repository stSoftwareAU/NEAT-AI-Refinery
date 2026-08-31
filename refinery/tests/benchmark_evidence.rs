//! The benchmark harness (issue #14).
//!
//! Refinery's performance claims are only worth what the measurement behind
//! them is worth, so the harness that produces the numbers is itself tested:
//! every case is measured through the real binary, every figure is read back
//! off the corpus that was actually published, and a run that fails to read
//! the whole corpus is a failure rather than a fast result.
//!
//! Every assertion below calls the real API — nothing here inspects source
//! text.
//!
//! The Deno comparison needs `deno` on `PATH`. Without it the one test that
//! uses it prints a skip notice and passes, matching `soak_evidence.rs`.

// The shared helpers serve several test binaries; not every one uses all of them.
#[allow(dead_code)]
mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::TempDir;
use neat_ai_refinery::bench::{
    bench, BenchCase, BenchConfig, BenchError, BenchReport, Comparison, DenoReference,
};
use neat_ai_refinery::corpus::RecordShape;

/// The binary under test, built by `cargo test`.
const BINARY: &str = env!("CARGO_BIN_EXE_neat_ai_refinery");

/// Two inputs and one output — twelve bytes a record, as the other suites use.
const INPUTS: usize = 2;
const OUTPUTS: usize = 1;

/// Corpus dimensions small enough that the suite runs in seconds.
const SHARDS: usize = 2;
const RECORDS_PER_SHARD: usize = 400;
const CORPUS_RECORDS: u64 = (SHARDS * RECORDS_PER_SHARD) as u64;

/// The rate every sampling case in these tests runs at.
const RATE: f64 = 0.5;

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

/// Is `deno` runnable? A missing `deno` skips, exactly as the soak harness.
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

/// The benchmark configuration the tests measure.
fn config(workspace: &Path, reference: Option<DenoReference>) -> BenchConfig {
    BenchConfig {
        workspace: workspace.to_path_buf(),
        binary: PathBuf::from(BINARY),
        shape: shape(),
        shards: SHARDS,
        records_per_shard: RECORDS_PER_SHARD,
        repeats: 2,
        cases: BenchCase::standard_suite(RATE),
        reference,
    }
}

/// One benchmark run over the small corpus, for the tests that only read it.
fn measured(name: &str) -> (TempDir, BenchReport) {
    let temp = TempDir::new(name);
    let report = bench(&config(temp.path(), None)).expect("run the benchmark");
    (temp, report)
}

#[test]
fn measures_every_case_in_the_standard_suite() {
    let (_temp, report) = measured("bench-suite");

    assert_eq!(
        report
            .cases
            .iter()
            .map(|case| case.label.as_str())
            .collect::<Vec<_>>(),
        vec!["sample", "quantise", "pipeline"],
        "the suite must cover the sample rate and the transform pipeline"
    );

    for case in &report.cases {
        assert!(
            case.elapsed_ms > 0,
            "{} was timed at zero milliseconds",
            case.label
        );
        assert_eq!(
            case.records_read, CORPUS_RECORDS,
            "{} did not read the whole corpus",
            case.label
        );
        assert_eq!(
            case.input_bytes, report.corpus.bytes,
            "{} read {} of the {} corpus bytes",
            case.label, case.input_bytes, report.corpus.bytes
        );
        assert!(case.output_bytes > 0, "{} published nothing", case.label);
        assert!(
            case.records_per_second() > 0.0,
            "{} reported no throughput",
            case.label
        );
        assert!(
            case.input_gib_per_second() > 0.0,
            "{} reported no read throughput",
            case.label
        );
        assert!(
            case.peak_rss_kib.is_some_and(|kib| kib > 0),
            "{} reported no peak memory",
            case.label
        );
        assert_eq!(case.repeats, 2, "{} was not repeated", case.label);
    }

    assert!(!report.host.os.is_empty(), "the host must be identified");
    assert_eq!(report.record_shape.inputs, INPUTS);
    assert!(report.reference.is_none(), "no reference was asked for");
}

#[test]
fn reports_the_output_size_each_transform_actually_published() {
    let (_temp, report) = measured("bench-output-size");

    let sample = report.case("sample").expect("the sample case was measured");
    assert!(
        sample.records_written > 0 && sample.records_written < CORPUS_RECORDS,
        "a rate of {RATE} kept {} of {CORPUS_RECORDS} records",
        sample.records_written
    );
    assert_eq!(
        sample.output_bytes,
        sample.records_written * shape().bytes_per_record() as u64,
        "the sample output size must be the records it kept"
    );

    let quantise = report
        .case("quantise")
        .expect("the quantise case was measured");
    assert_eq!(
        quantise.records_written, CORPUS_RECORDS,
        "quantisation keeps every record"
    );
    assert_eq!(
        quantise.output_bytes,
        quantise.input_bytes / 2,
        "bfloat16 halves the corpus"
    );
    assert!(
        (quantise.output_ratio() - 0.5).abs() < 1e-9,
        "quantise reported an output ratio of {}",
        quantise.output_ratio()
    );
}

#[test]
fn renders_as_committable_evidence() {
    let (_temp, report) = measured("bench-report");

    let json = report.to_json().expect("encode the report");
    let markdown = report.to_markdown();

    let parsed: serde_json::Value = serde_json::from_str(&json).expect("the report is JSON");
    assert_eq!(parsed["host"]["os"], serde_json::json!(report.host.os));
    assert_eq!(parsed["cases"].as_array().expect("cases").len(), 3);

    assert!(markdown.contains("| case |"), "{markdown}");
    assert!(markdown.contains(&report.host.arch), "{markdown}");
    assert!(
        markdown.contains("input GiB/s") && markdown.contains("peak RSS"),
        "the evidence must name the metrics it reports: {markdown}"
    );
    assert!(
        markdown.contains("records/s"),
        "the evidence must report records/s: {markdown}"
    );

    let round_tripped: BenchReport = serde_json::from_str(&json).expect("the report round-trips");
    assert_eq!(round_tripped.cases.len(), report.cases.len());
    assert_eq!(round_tripped.corpus.bytes, report.corpus.bytes);
}

#[test]
fn refuses_a_configuration_that_would_measure_nothing() {
    let temp = TempDir::new("bench-refusals");

    let mut zero_repeats = config(temp.path(), None);
    zero_repeats.repeats = 0;
    let error = bench(&zero_repeats).expect_err("zero repeats measure nothing");
    assert!(matches!(error, BenchError::Config { .. }), "{error:?}");

    let mut no_cases = config(temp.path(), None);
    no_cases.cases = Vec::new();
    let error = bench(&no_cases).expect_err("a benchmark of no cases measures nothing");
    assert!(matches!(error, BenchError::Config { .. }), "{error:?}");
}

#[test]
fn fails_loud_when_the_binary_under_measurement_cannot_run() {
    let temp = TempDir::new("bench-missing-binary");
    let mut broken = config(temp.path(), None);
    broken.binary = PathBuf::from("/does/not/exist/neat_ai_refinery");

    let error = bench(&broken).expect_err("a binary that cannot be spawned is not a fast run");

    assert!(matches!(error, BenchError::Measure { .. }), "{error:?}");
}

#[test]
fn a_baseline_comparison_passes_a_run_that_held_its_ground() {
    let (_temp, report) = measured("bench-baseline-clean");

    let comparison =
        Comparison::of(&report, &report, 0.25).expect("a report is comparable with itself");

    assert!(
        comparison.is_clean(),
        "a run compared with itself regressed: {:?}",
        comparison.regressions()
    );
    comparison
        .assert_clean()
        .expect("a clean comparison must not fail the gate");
    assert!(comparison.to_markdown().contains("| case |"));
}

#[test]
fn a_baseline_comparison_fails_loud_on_a_throughput_regression() {
    let (_temp, baseline) = measured("bench-baseline-regression");
    let mut slower = baseline.clone();
    for case in &mut slower.cases {
        case.elapsed_ms = case.elapsed_ms * 4 + 4;
    }

    let comparison = Comparison::of(&baseline, &slower, 0.25).expect("comparable reports");

    assert!(!comparison.is_clean(), "a four-times slower run passed");
    let regressions = comparison.regressions();
    assert_eq!(
        regressions.len(),
        baseline.cases.len(),
        "every case regressed: {regressions:?}"
    );
    assert!(
        regressions.iter().any(|line| line.contains("sample")),
        "{regressions:?}"
    );
    let error = comparison
        .assert_clean()
        .expect_err("a regression must fail the gate");
    assert!(matches!(error, BenchError::Regression { .. }), "{error:?}");
}

#[test]
fn a_baseline_comparison_flags_a_case_that_stopped_being_measured() {
    let (_temp, baseline) = measured("bench-baseline-missing");
    let mut narrowed = baseline.clone();
    narrowed.cases.retain(|case| case.label != "quantise");

    let comparison = Comparison::of(&baseline, &narrowed, 0.25).expect("comparable reports");

    assert!(!comparison.is_clean(), "a lost case is a lost gate");
    assert!(
        comparison
            .regressions()
            .iter()
            .any(|line| line.contains("quantise")),
        "{:?}",
        comparison.regressions()
    );
}

#[test]
fn a_baseline_comparison_refuses_a_baseline_of_another_corpus() {
    let (_temp, baseline) = measured("bench-baseline-incomparable");
    let mut other = baseline.clone();
    other.corpus.records_per_shard *= 2;
    other.corpus.bytes *= 2;

    let error =
        Comparison::of(&baseline, &other, 0.25).expect_err("two different corpora do not compare");

    assert!(matches!(error, BenchError::Config { .. }), "{error:?}");
}

#[test]
fn a_baseline_comparison_refuses_a_tolerance_it_cannot_apply() {
    let (_temp, report) = measured("bench-baseline-tolerance");

    for tolerance in [-0.1, 1.0, f64::NAN] {
        let error = Comparison::of(&report, &report, tolerance)
            .expect_err("a tolerance outside [0, 1) is not a gate");
        assert!(matches!(error, BenchError::Config { .. }), "{error:?}");
    }
}

#[test]
fn the_speedup_gate_needs_a_reference_to_gate_on() {
    let (_temp, report) = measured("bench-speedup-unmeasured");

    assert!(report.sample_speedup().is_none());
    let error = report
        .check_speedup(1.5)
        .expect_err("a gate with nothing to compare against must not pass");
    assert!(matches!(error, BenchError::Config { .. }), "{error:?}");
}

#[test]
fn compares_refinery_against_the_deno_sampler_and_gates_on_it() {
    if !deno_available("compares_refinery_against_the_deno_sampler_and_gates_on_it") {
        return;
    }
    let temp = TempDir::new("bench-reference");
    let reference = DenoReference {
        parity_dir: parity_dir(),
        rate: RATE,
    };

    let report = bench(&config(temp.path(), Some(reference))).expect("run the benchmark");

    let deno = report
        .reference
        .as_ref()
        .expect("the Deno sampler was measured");
    assert_eq!(deno.mirrors, "sample");
    assert!(deno.result.elapsed_ms > 0, "the Deno run was not timed");
    assert_eq!(
        deno.result.records_read, CORPUS_RECORDS,
        "the reference must read the same corpus"
    );
    assert!(
        deno.result.output_bytes > 0,
        "the reference published nothing"
    );

    let speedup = report
        .sample_speedup()
        .expect("both samplers were measured");
    assert!(speedup > 0.0, "the speedup was reported as {speedup}");
    assert!(
        report.to_markdown().contains("typescript"),
        "the evidence must carry the comparison: {}",
        report.to_markdown()
    );

    // The gate is the one CI enforces: a floor it must clear, not a claim.
    let unreachable = speedup * 100.0;
    let error = report
        .check_speedup(unreachable)
        .expect_err("a gate above the measured speedup must fail loud");
    assert!(matches!(error, BenchError::Regression { .. }), "{error:?}");
    report
        .check_speedup(speedup / 2.0)
        .expect("a gate below the measured speedup passes");
}
