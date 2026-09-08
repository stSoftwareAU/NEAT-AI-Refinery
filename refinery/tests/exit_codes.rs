//! The exit codes a failed run reports.
//!
//! A caller retries a run that failed because the target volume filled up, and
//! does not retry one that failed for any other reason, so "the disk is full"
//! has to be distinguishable from the outside — see
//! [`docs/grq-integration.md`](../../docs/grq-integration.md). These tests
//! drive real out-of-space failures rather than synthesised ones wherever the
//! platform can produce them: writing to `/dev/full` raises a genuine `ENOSPC`
//! from the kernel, through the same [`RecordWriter`] every transform writes a
//! derived corpus with.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use neat_ai_refinery::cli::CliError;
use neat_ai_refinery::corpus::{CorpusError, DerivedDestination, RecordShape, RecordWriter};
use neat_ai_refinery::exit::{code_for, failure_report, FAILURE, STORAGE_FULL};
use neat_ai_refinery::manifest::CallerMetadata;
use neat_ai_refinery::pipeline::{PipelineError, StageError};
use neat_ai_refinery::sample::{whole_pass_bytes, SampleError, SampleRate, SampleRequest};

mod common;

use common::{encode, TempDir};

/// The token GRQ's `worker/shared/sampler_enospc.sh` greps out of the run log.
const REQUIREMENT_TOKEN: &str = "required_bytes=";

/// The binary under test, built by Cargo for this integration test.
const BINARY: &str = env!("CARGO_BIN_EXE_neat_ai_refinery");

/// A device that accepts an open and fails every write with `ENOSPC`.
const FULL_DEVICE: &str = "/dev/full";

/// Whether this platform offers a device that reports a full volume.
///
/// Linux always does, and that is where CI runs, so a missing device there is a
/// broken host rather than a reason to report a green test that asserted
/// nothing: only a platform without the device at all skips these tests.
fn has_full_device() -> bool {
    let present = PathBuf::from(FULL_DEVICE).exists();
    assert!(
        present || !cfg!(target_os = "linux"),
        "{FULL_DEVICE} is missing on a Linux host, so the out-of-space path \
         cannot be exercised — fix the host rather than skipping it"
    );
    present
}

/// Writes one record to `/dev/full` and returns the failure the kernel raised.
fn write_to_a_full_volume() -> CorpusError {
    let shape = RecordShape::new(2, 1).expect("a two-input, one-output record shape");
    let sources = vec![PathBuf::from("/dev/null")];
    let destination =
        DerivedDestination::new(FULL_DEVICE, &sources).expect("the full device is a destination");

    let mut writer = RecordWriter::create(&destination, shape).expect("open the full device");
    let mut failure = writer.write_values(&[1.0, 2.0, 3.0]).err();
    failure = failure.or_else(|| writer.finish().err());

    failure.expect("a write to a full volume fails")
}

#[test]
fn a_full_volume_exits_with_the_enospc_code() {
    if !has_full_device() {
        eprintln!("skipped: {FULL_DEVICE} is not available on this platform");
        return;
    }

    let error = CliError::Sample(SampleError::Corpus(write_to_a_full_volume()));

    assert_eq!(
        code_for(&error),
        STORAGE_FULL,
        "a full volume reports POSIX ENOSPC so a caller can retry it: {error}"
    );
}

#[test]
fn a_reported_write_failure_does_not_panic_when_the_writer_is_dropped() {
    if !has_full_device() {
        eprintln!("skipped: {FULL_DEVICE} is not available on this platform");
        return;
    }

    // The failure was already reported to the caller, so dropping the writer
    // that raised it must not panic the process over the same loss — a panic
    // would replace the mapped exit code with an abort.
    let error = write_to_a_full_volume();

    assert!(
        matches!(error, CorpusError::Io { .. }),
        "a full volume is an I/O failure: {error}"
    );
}

#[test]
fn a_full_volume_deep_in_a_pipeline_stage_still_exits_with_the_enospc_code() {
    let error = CliError::Pipeline(PipelineError::Stage {
        position: 2,
        name: "quantise".to_string(),
        source: Box::new(StageError::Sample(SampleError::Io {
            path: PathBuf::from("/data/trainData-binary-sampler/sample-5.bin"),
            source: io::Error::from_raw_os_error(28),
        })),
    });

    assert_eq!(
        code_for(&error),
        STORAGE_FULL,
        "the code is found however deeply the failure is wrapped: {error}"
    );
}

#[test]
fn the_out_of_space_code_is_the_posix_number_callers_gate_on() {
    // A caller's retry gate matches the number, not the name: GRQ's
    // `worker/shared/sampler_enospc.sh` retries a sampler run that exited 28,
    // and it cannot see this crate's constant. Changing either number without
    // the other silently unhooks the gate, so the wire values are pinned here.
    assert_eq!(STORAGE_FULL, 28, "ENOSPC is 28 — a caller gates on it");
    assert_eq!(FAILURE, 1, "every other failure keeps the ordinary code");
}

#[test]
fn every_other_failure_keeps_the_ordinary_exit_code() {
    let refused = CliError::Sample(SampleError::InvalidRate { rate: 1.5 });
    let unwritable = CliError::Sample(SampleError::Io {
        path: PathBuf::from("/data/trainData-binary-sampler"),
        source: io::Error::from(io::ErrorKind::PermissionDenied),
    });

    assert_eq!(code_for(&refused), FAILURE, "{refused}");
    assert_eq!(code_for(&unwritable), FAILURE, "{unwritable}");
}

#[test]
fn the_binary_reports_an_ordinary_failure_as_exit_one() {
    let directory = TempDir::new("exit-codes");
    let source = directory.path().join("source");
    fs::create_dir_all(&source).expect("create the source directory");
    directory.write("source/shard-1.bin", &encode(&[1.0, 2.0, 3.0]));

    let status = Command::new(BINARY)
        .args([
            "--source".as_ref(),
            source.as_os_str(),
            "--output".as_ref(),
            directory.path().join("derived").as_os_str(),
            "--inputs".as_ref(),
            "2".as_ref(),
            "--outputs".as_ref(),
            "1".as_ref(),
            "sample".as_ref(),
            "--rate".as_ref(),
            "1.5".as_ref(),
        ])
        .status()
        .expect("run the binary");

    assert_eq!(
        status.code(),
        Some(i32::from(FAILURE)),
        "a rate outside (0, 1] is an ordinary failure, not a full volume"
    );
}

/// The `required_bytes` figures a report carries, one per line that names one.
///
/// The token is read exactly as GRQ reads it — `required_bytes=` followed by
/// ASCII digits, matched anywhere on the line — so a change of shape here
/// fails this test rather than silently blinding the caller.
fn reported_requirements(report: &str) -> Vec<u64> {
    report
        .lines()
        .filter_map(|line| line.split_once(REQUIREMENT_TOKEN))
        .map(|(_, rest)| {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            assert_eq!(
                digits, rest,
                "the token carries ASCII digits and nothing else: {rest}"
            );
            digits.parse::<u64>().expect("the figure is an integer")
        })
        .collect()
}

/// A source corpus of two files, holding `first` and `second` records of the
/// three-value shape these tests use, and the request that samples it.
fn two_file_request(directory: &TempDir, rate: f64, first: usize, second: usize) -> SampleRequest {
    let source = directory.path().join("source");
    fs::create_dir_all(&source).expect("create the source directory");
    directory.write("source/shard-1.bin", &encode(&vec![1.0; first * 3]));
    directory.write("source/shard-2.bin", &encode(&vec![2.0; second * 3]));

    SampleRequest {
        source,
        output: directory.path().join("derived"),
        shape: RecordShape::new(2, 1).expect("a three-value record shape"),
        rate: SampleRate::new(rate).expect("a valid rate"),
        seed: Some(20_260_908),
        metadata: CallerMetadata::default(),
    }
}

#[test]
fn a_full_volume_reports_what_a_whole_fresh_attempt_needs() {
    if !has_full_device() {
        eprintln!("skipped: {FULL_DEVICE} is not available on this platform");
        return;
    }

    // A real corpus, and the real kernel ENOSPC a write to a full volume
    // raises — the estimate and the failure both come from the code under
    // test rather than from a fixture.
    let directory = TempDir::new("required-bytes");
    let request = two_file_request(&directory, 0.05, 40, 60);
    let source_bytes = 100.0_f64 * 12.0;
    let required = whole_pass_bytes(&request).expect("estimate a whole pass");

    let error = CliError::Sample(
        SampleError::Corpus(write_to_a_full_volume()).with_required_bytes(required),
    );
    let report = failure_report(&error);
    let reported = reported_requirements(&report);

    assert_eq!(
        code_for(&error),
        STORAGE_FULL,
        "reporting the requirement does not change the code a caller gates on: {report}"
    );
    assert_eq!(
        reported.len(),
        1,
        "exactly one figure is reported, so a caller reads one number: {report}"
    );
    let figure = reported[0];
    assert!(
        figure >= (source_bytes * 0.05).ceil() as u64,
        "the figure covers a whole pass, not the remainder of the failed one: {figure}"
    );
    assert!(
        figure <= (source_bytes * 0.05 * 1.02).ceil() as u64,
        "the headroom stays within 2%, so a caller is not sent hunting for space \
         it does not need: {figure}"
    );
    assert!(
        report.starts_with("neat_ai_refinery: "),
        "the failure itself is still the first line: {report}"
    );
}

#[test]
fn an_ordinary_failure_reports_no_requirement() {
    // Exit 1 repeats however much space is freed, so a figure beside it would
    // buy a caller a retry that cannot succeed.
    let directory = TempDir::new("exit-one-no-requirement");
    let source = directory.path().join("source");
    fs::create_dir_all(&source).expect("create the source directory");
    directory.write("source/notes.txt", b"no corpus files here");

    let output = Command::new(BINARY)
        .args([
            "--source".as_ref(),
            source.as_os_str(),
            "--output".as_ref(),
            directory.path().join("derived").as_os_str(),
            "--inputs".as_ref(),
            "2".as_ref(),
            "--outputs".as_ref(),
            "1".as_ref(),
            "sample".as_ref(),
            "--rate".as_ref(),
            "0.05".as_ref(),
        ])
        .output()
        .expect("run the binary");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    assert_eq!(
        output.status.code(),
        Some(i32::from(FAILURE)),
        "a source with no .bin files is an ordinary failure: {stderr}"
    );
    assert!(
        stderr.contains("holds no .bin corpus files"),
        "the failure is still reported in full: {stderr}"
    );
    assert!(
        reported_requirements(&stderr).is_empty(),
        "no figure is printed for a failure a retry cannot fix: {stderr}"
    );
}

#[test]
fn a_requirement_survives_the_wrapping_a_pipeline_stage_adds() {
    // A sampling stage inside a pipeline fills the same volume, and the figure
    // has to reach the caller through the stage that reports it.
    let error = CliError::Pipeline(PipelineError::Stage {
        position: 1,
        name: "sample".to_string(),
        source: Box::new(StageError::Sample(
            SampleError::Io {
                path: PathBuf::from("/data/trainData-binary-sampler/sample-5.bin"),
                source: io::Error::from_raw_os_error(28),
            }
            .with_required_bytes(8_080_000),
        )),
    });

    let report = failure_report(&error);

    assert_eq!(code_for(&error), STORAGE_FULL, "{report}");
    assert_eq!(reported_requirements(&report), vec![8_080_000]);
}

/// The script that runs the binary with its output on a volume too small to
/// hold the sample: a tiny `tmpfs` inside a private mount namespace, which is
/// the only way to fill a volume without privileges. Exit 90 says the host
/// refused the namespace or the mount.
const FULL_VOLUME_SCRIPT: &str = r#"
set -u
mount_point="$1"
binary="$2"
source_dir="$3"
mkdir -p "$mount_point" || exit 90
mount -t tmpfs -o size=8k tmpfs "$mount_point" 2>/dev/null || exit 90
"$binary" --source "$source_dir" --output "$mount_point/derived" \
  --inputs 2 --outputs 1 sample --rate 1.0 --seed 1
"#;

/// Exit code the script reports when the host would not give it a full volume.
const NO_FULL_VOLUME: i32 = 90;

/// Samples `source` onto a volume too small for the result, returning the
/// binary's exit code and stderr.
///
/// `None` means this host does not offer an unprivileged mount namespace, so
/// the run could not be driven at all — the caller says so out loud rather
/// than reporting a test that asserted nothing as a pass.
fn sample_onto_a_full_volume(directory: &TempDir, source: &Path) -> Option<(i32, String)> {
    let output = Command::new("unshare")
        .args(["--user", "--map-root-user", "--mount"])
        .args([
            OsStr::new("bash"),
            OsStr::new("-c"),
            OsStr::new(FULL_VOLUME_SCRIPT),
            // `bash -c script name …` names the script, so the volume is `$1`.
            OsStr::new("neat-ai-refinery-full-volume"),
            directory.path().join("volume").as_os_str(),
            OsStr::new(BINARY),
            source.as_os_str(),
        ])
        .output()
        .ok()?;

    let code = output.status.code()?;
    if code == NO_FULL_VOLUME {
        return None;
    }
    Some((code, String::from_utf8_lossy(&output.stderr).into_owned()))
}

#[test]
fn the_binary_reports_the_requirement_when_a_real_volume_fills_up() {
    let directory = TempDir::new("full-volume-binary");
    // 1000 records of three `f32` values — 12 000 bytes, more than the 8 KiB
    // volume the run publishes onto, so the kernel raises a genuine ENOSPC.
    let request = two_file_request(&directory, 1.0, 500, 500);
    let source_bytes = 1000.0_f64 * 12.0;

    let Some((code, stderr)) = sample_onto_a_full_volume(&directory, &request.source) else {
        eprintln!(
            "skipped: this host offers no unprivileged mount namespace, so no volume \
             can be filled — the reporting itself is still covered by \
             `a_full_volume_reports_what_a_whole_fresh_attempt_needs`"
        );
        return;
    };

    let reported = reported_requirements(&stderr);
    assert_eq!(
        code,
        i32::from(STORAGE_FULL),
        "a full volume still exits 28: {stderr}"
    );
    assert_eq!(
        reported.len(),
        1,
        "the run reports exactly one figure on stderr: {stderr}"
    );
    assert!(
        reported[0] >= source_bytes.ceil() as u64
            && reported[0] <= (source_bytes * 1.02).ceil() as u64,
        "a rate of 1 needs the whole source plus its headroom: {stderr}"
    );
}
