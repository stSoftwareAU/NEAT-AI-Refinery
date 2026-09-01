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

use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Command;

use neat_ai_refinery::cli::CliError;
use neat_ai_refinery::corpus::{CorpusError, DerivedDestination, RecordShape, RecordWriter};
use neat_ai_refinery::exit::{code_for, FAILURE, STORAGE_FULL};
use neat_ai_refinery::pipeline::{PipelineError, StageError};
use neat_ai_refinery::sample::SampleError;

mod common;

use common::{encode, TempDir};

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
fn a_full_volume_names_the_space_the_pass_still_needs() {
    // The exit code says the volume is full; it cannot say whether another
    // attempt would fit. GRQ's retry gate spent three attempts on a volume with
    // 19 GB free because nothing said the pass needed about 19 GB
    // (stSoftwareAU/GRQ#4611), so the failure now names it — and the gate reads
    // the figure out of the message by that spelling, which pins it here.
    let error = CliError::Sample(SampleError::StorageFull {
        required_bytes: 61_440,
        records_written: 4_485,
        records_expected: 7_426,
        source: Box::new(SampleError::Io {
            path: PathBuf::from("/data/trainData-binary-sampler/sample-5.bin"),
            source: io::Error::from_raw_os_error(28),
        }),
    });

    let message = format!("{error}");
    assert!(
        message.contains("required_bytes=61440"),
        "a caller reads the requirement by this spelling: {message}"
    );
    assert_eq!(
        code_for(&error),
        STORAGE_FULL,
        "the figures are additional — a full volume still exits 28: {message}"
    );
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
