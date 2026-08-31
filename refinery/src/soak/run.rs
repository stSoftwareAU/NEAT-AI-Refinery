//! The soak run itself.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use super::{
    AtomicPublicationEvidence, ConsumerCheck, CorpusDigest, CorpusFacts, HostFacts,
    MeasuredCommand, PublishedCorpus, RunMeasurement, SoakError, SoakReport, SoakRound,
};
use crate::corpus::{write_synthetic_corpus, RecordShape};
use crate::manifest::ToolIdentity;
use crate::sample::SampleRate;

/// The Deno half of the soak: the reference sampler, and optionally the
/// NEAT-AI consumer check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenoReference {
    /// The `parity/` directory holding `grq_sampler.ts` and `evolve_dir.ts`.
    pub parity_dir: PathBuf,
    /// Also run `evolve_dir.ts` over the corpus Refinery published.
    ///
    /// NEAT-AI fetches its WASM bundle from `jsr.io` the first time it runs on
    /// a host, so this needs network access on a cold machine.
    pub check_consumer: bool,
}

/// What to soak, and how hard.
#[derive(Debug, Clone, PartialEq)]
pub struct SoakConfig {
    /// A scratch directory the corpora and captured output are built in.
    pub workspace: PathBuf,
    /// The `neat_ai_refinery` binary to soak — the one production would run.
    pub binary: PathBuf,
    /// The record shape both samplers read the corpus with.
    pub shape: RecordShape,
    /// Corpus files to build.
    pub shards: usize,
    /// Records in each of them.
    pub records_per_shard: usize,
    /// The sampling rate.
    pub rate: SampleRate,
    /// Sampling rounds to measure.
    pub rounds: usize,
    /// The Deno comparison, when this host can run it.
    pub reference: Option<DenoReference>,
}

/// Runs one soak and returns the evidence it produced.
///
/// # Errors
///
/// Returns [`SoakError::Invariant`] when the source corpus changed, a
/// published corpus failed verification, or a run that must fail did not;
/// [`SoakError::CommandFailed`] when a sampling round failed; and
/// [`SoakError::Io`] when the workspace cannot be prepared.
pub fn soak(config: &SoakConfig) -> Result<SoakReport, SoakError> {
    let workspace = &config.workspace;
    let logs = workspace.join("logs");
    fs::create_dir_all(&logs).map_err(|e| SoakError::io(&logs, e))?;

    let source = workspace.join("trainData-binary");
    let output = workspace.join("trainData-binary-sampler");
    let corpus_bytes = write_synthetic_corpus(
        &source,
        config.shards,
        config.records_per_shard,
        config.shape,
    )?;
    let before = CorpusDigest::of(&source)?;

    let mut rounds = Vec::with_capacity(config.rounds);
    for round in 1..=config.rounds {
        let measurement = MeasuredCommand::new(format!("refinery-{round}"), &config.binary, &logs)
            .args(sampler_args(&source, &output, config))
            .measure()?;
        let published = PublishedCorpus::verify(&output, config.shape)?;
        rounds.push(SoakRound {
            round,
            measurement,
            published,
        });
    }

    let after = CorpusDigest::of(&source)?;
    if after != before {
        return Err(SoakError::invariant(
            "source immutability",
            format!("{} was modified by a sampling run", source.display()),
        ));
    }

    let (reference, reference_records_written) = match &config.reference {
        Some(deno) => {
            let (measurement, kept) = measure_reference(deno, &source, workspace, &logs, config)?;
            (Some(measurement), Some(kept))
        }
        None => (None, None),
    };
    let consumer = match &config.reference {
        Some(deno) if deno.check_consumer => Some(check_consumer(deno, &output, &logs, config)?),
        _ => None,
    };

    let atomic_publication = check_atomic_publication(&output, workspace, &logs, config)?;

    Ok(SoakReport {
        tool: ToolIdentity::current(),
        host: HostFacts::detect(),
        record_shape: config.shape.into(),
        rate: config.rate.value(),
        corpus: CorpusFacts {
            shards: config.shards,
            records_per_shard: config.records_per_shard,
            bytes: corpus_bytes,
        },
        rounds,
        reference,
        reference_records_written,
        consumer,
        source_unchanged: true,
        atomic_publication,
    })
}

/// The command line GRQ runs in production, minus the seed.
fn sampler_args(source: &Path, output: &Path, config: &SoakConfig) -> Vec<OsString> {
    let host = HostFacts::detect();
    vec![
        OsString::from("--source"),
        source.into(),
        OsString::from("--output"),
        output.into(),
        OsString::from("--inputs"),
        config.shape.inputs().to_string().into(),
        OsString::from("--outputs"),
        config.shape.outputs().to_string().into(),
        OsString::from("--metadata"),
        format!("soak_host={}-{}", host.os, host.arch).into(),
        OsString::from("sample"),
        OsString::from("--rate"),
        config.rate.value().to_string().into(),
    ]
}

/// Measures the Deno sampler over the same corpus, into its own directory.
fn measure_reference(
    deno: &DenoReference,
    source: &Path,
    workspace: &Path,
    logs: &Path,
    config: &SoakConfig,
) -> Result<(RunMeasurement, u64), SoakError> {
    let output = workspace.join("reference-sampler");
    let command = MeasuredCommand::new("typescript", "deno", logs)
        .current_dir(&deno.parity_dir)
        .args(["run", "--allow-read", "--allow-write", "grq_sampler.ts"])
        .arg("--source")
        .arg(source)
        .arg("--output")
        .arg(&output)
        .args(["--inputs", &config.shape.inputs().to_string()])
        .args(["--outputs", &config.shape.outputs().to_string()])
        .args(["--rate", &config.rate.value().to_string()]);

    let measurement = command.measure()?;
    let summary = last_json_line(&command.stdout_path())?;
    let kept = summary
        .get("recordsWritten")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            SoakError::invariant(
                "reference summary",
                format!("the Deno sampler did not report recordsWritten: {summary}"),
            )
        })?;

    Ok((measurement, kept))
}

/// Re-runs NEAT-AI's `evolveDir` over the corpus Refinery published.
fn check_consumer(
    deno: &DenoReference,
    output: &Path,
    logs: &Path,
    config: &SoakConfig,
) -> Result<ConsumerCheck, SoakError> {
    let command = MeasuredCommand::new("evolve-dir", "deno", logs)
        .current_dir(&deno.parity_dir)
        .args([
            "run",
            "--allow-read",
            "--allow-write",
            "--allow-env",
            "--allow-run",
            "--allow-sys",
            "--allow-net=jsr.io",
            "evolve_dir.ts",
        ])
        .arg("--corpus")
        .arg(output)
        .args(["--inputs", &config.shape.inputs().to_string()])
        .args(["--outputs", &config.shape.outputs().to_string()]);

    command.measure()?;
    let summary = last_json_line(&command.stdout_path())?;
    let consumed = summary
        .get("consumed")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !consumed {
        return Err(SoakError::invariant(
            "consumer",
            format!("evolveDir did not consume {}: {summary}", output.display()),
        ));
    }

    Ok(ConsumerCheck {
        consumed,
        summary: summary.to_string(),
    })
}

/// Proves a failed run leaves the published corpus exactly as it was.
///
/// The failure is caused by a corpus ending mid-record — a real fatal
/// condition, in a throwaway source directory. A full volume would be the
/// other way to provoke it, but it cannot be simulated portably without
/// privileges the soak deliberately does not ask for.
fn check_atomic_publication(
    output: &Path,
    workspace: &Path,
    logs: &Path,
    config: &SoakConfig,
) -> Result<AtomicPublicationEvidence, SoakError> {
    let published_before = CorpusDigest::of(output)?;

    let broken = workspace.join("broken-source");
    fs::create_dir_all(&broken).map_err(|e| SoakError::io(&broken, e))?;
    let partial = vec![0_u8; config.shape.bytes_per_record() + 1];
    let shard = broken.join("shard-000.bin");
    fs::write(&shard, &partial).map_err(|e| SoakError::io(&shard, e))?;

    let failure = MeasuredCommand::new("refinery-failure", &config.binary, logs)
        .args(sampler_args(&broken, output, config))
        .measure();
    let failed_run_exit_code = match failure {
        Err(SoakError::CommandFailed { code, .. }) => code,
        Err(other) => return Err(other),
        Ok(_) => {
            return Err(SoakError::invariant(
                "fail loud",
                format!(
                    "{} published a corpus from a source ending mid-record",
                    config.binary.display()
                ),
            ))
        }
    };

    let published_after = CorpusDigest::of(output)?;
    if published_after != published_before {
        return Err(SoakError::invariant(
            "atomic publication",
            format!(
                "a failed run changed the corpus published at {}",
                output.display()
            ),
        ));
    }

    Ok(AtomicPublicationEvidence {
        failed_run_exit_code,
        previous_corpus_intact: true,
        scratch_left_behind: count_scratch(workspace)?,
    })
}

/// Staging and aside directories left in `directory`.
fn count_scratch(directory: &Path) -> Result<usize, SoakError> {
    let mut left = 0;
    for entry in fs::read_dir(directory).map_err(|e| SoakError::io(directory, e))? {
        let entry = entry.map_err(|e| SoakError::io(directory, e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.contains(".staging-") || name.contains(".deleting-") {
            left += 1;
        }
    }
    Ok(left)
}

/// The last JSON object a harness printed on standard output.
fn last_json_line(path: &Path) -> Result<serde_json::Value, SoakError> {
    let captured = fs::read_to_string(path).map_err(|e| SoakError::io(path, e))?;
    let line = captured.lines().rev().find(|line| !line.trim().is_empty());
    match line {
        Some(line) => Ok(serde_json::from_str(line)?),
        None => Err(SoakError::invariant(
            "harness summary",
            format!("{} reported nothing on standard output", path.display()),
        )),
    }
}
