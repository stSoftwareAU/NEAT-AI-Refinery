//! The benchmark run itself.
//!
//! One corpus, built once, read by every case through the real binary — the
//! build a caller would run, not an in-process shortcut, so the figures
//! include process start-up and the published manifest is available to read
//! the output size back off.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use super::{
    BenchCase, BenchError, BenchReport, BenchTransform, CaseResult, CorpusFacts, Reference,
};
use crate::corpus::{write_synthetic_corpus, RecordShape};
use crate::manifest::{Manifest, ToolIdentity, MANIFEST_FILE_NAME};
use crate::soak::{HostFacts, MeasuredCommand, RunMeasurement};

/// The Deno sampler to measure beside Refinery.
#[derive(Debug, Clone, PartialEq)]
pub struct DenoReference {
    /// The `parity/` directory holding `grq_sampler.ts`.
    pub parity_dir: PathBuf,
    /// The rate it samples at — the rate of the Refinery case it mirrors.
    pub rate: f64,
}

/// What to benchmark, and over how much.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchConfig {
    /// A scratch directory the corpus and captured output are built in.
    pub workspace: PathBuf,
    /// The `neat_ai_refinery` binary to measure — the one production runs.
    pub binary: PathBuf,
    /// The record shape every case reads the corpus with.
    pub shape: RecordShape,
    /// Corpus files to build.
    pub shards: usize,
    /// Records in each of them.
    pub records_per_shard: usize,
    /// How many times each case is run; the fastest run is reported.
    pub repeats: usize,
    /// The cases to measure, in order.
    pub cases: Vec<BenchCase>,
    /// The Deno comparison, when this host can run it.
    pub reference: Option<DenoReference>,
}

/// Runs one benchmark and returns the evidence it produced.
///
/// # Errors
///
/// Returns [`BenchError::Config`] when the run would measure nothing or the
/// reference has no case to mirror, [`BenchError::Measure`] when a measured
/// process cannot be started or fails, [`BenchError::Invariant`] when a case
/// did not read the whole corpus or published bytes that disagree with its own
/// manifest, and [`BenchError::Io`] when the workspace cannot be prepared.
pub fn bench(config: &BenchConfig) -> Result<BenchReport, BenchError> {
    if config.repeats == 0 {
        return Err(BenchError::config(
            "a benchmark of zero repeats measures nothing",
        ));
    }
    if config.cases.is_empty() {
        return Err(BenchError::config(
            "a benchmark of no cases measures nothing",
        ));
    }

    let workspace = &config.workspace;
    let logs = workspace.join("logs");
    fs::create_dir_all(&logs).map_err(|e| BenchError::io(&logs, e))?;

    let source = workspace.join("trainData-binary");
    let corpus_bytes = write_synthetic_corpus(
        &source,
        config.shards,
        config.records_per_shard,
        config.shape,
    )?;
    let corpus_records = (config.shards as u64) * (config.records_per_shard as u64);

    let mut cases = Vec::with_capacity(config.cases.len());
    for case in &config.cases {
        cases.push(measure_case(case, &source, &logs, config, corpus_records)?);
    }

    let reference = match &config.reference {
        Some(deno) => Some(measure_reference(
            deno,
            &source,
            &logs,
            config,
            corpus_bytes,
            corpus_records,
        )?),
        None => None,
    };

    Ok(BenchReport {
        tool: ToolIdentity::current(),
        host: HostFacts::detect(),
        record_shape: config.shape.into(),
        corpus: CorpusFacts {
            shards: config.shards,
            records_per_shard: config.records_per_shard,
            bytes: corpus_bytes,
        },
        repeats: config.repeats,
        cases,
        reference,
    })
}

/// Runs one case `repeats` times and reads its figures off what it published.
fn measure_case(
    case: &BenchCase,
    source: &Path,
    logs: &Path,
    config: &BenchConfig,
    corpus_records: u64,
) -> Result<CaseResult, BenchError> {
    let output = config.workspace.join(format!("out-{}", case.label));
    let args = transform_args(case, source, &output, config)?;

    let mut fastest: Option<RunMeasurement> = None;
    let mut peak_rss_kib: Option<u64> = None;
    let mut peak_rss_method = String::new();
    for repeat in 1..=config.repeats {
        let measurement =
            MeasuredCommand::new(format!("{}-{repeat}", case.label), &config.binary, logs)
                .args(args.clone())
                .measure()?;
        peak_rss_kib = match (peak_rss_kib, measurement.peak_rss_kib) {
            (Some(seen), Some(sample)) => Some(seen.max(sample)),
            (seen, sample) => seen.or(sample),
        };
        peak_rss_method.clone_from(&measurement.peak_rss_method);
        if fastest
            .as_ref()
            .is_none_or(|best| measurement.elapsed_ms < best.elapsed_ms)
        {
            fastest = Some(measurement);
        }
    }
    let fastest = fastest.ok_or_else(|| {
        BenchError::invariant("repeats", "no run was measured, despite repeats above zero")
    })?;

    let manifest = Manifest::load(output.join(MANIFEST_FILE_NAME))?;
    if manifest.source.record_count != corpus_records {
        return Err(BenchError::invariant(
            "corpus coverage",
            format!(
                "{} read {} of the {corpus_records} records in the corpus",
                case.label, manifest.source.record_count
            ),
        ));
    }
    let published = output.join(&manifest.output.file);
    let on_disk = fs::metadata(&published)
        .map_err(|e| BenchError::io(&published, e))?
        .len();
    if on_disk != manifest.output.bytes {
        return Err(BenchError::invariant(
            "published bytes",
            format!(
                "{} holds {on_disk} bytes, its manifest records {}",
                published.display(),
                manifest.output.bytes
            ),
        ));
    }

    Ok(CaseResult {
        label: case.label.clone(),
        transform: case.description(),
        repeats: config.repeats,
        elapsed_ms: fastest.elapsed_ms,
        peak_rss_kib,
        peak_rss_method,
        input_bytes: manifest.source.files.iter().map(|file| file.bytes).sum(),
        output_bytes: manifest.output.bytes,
        records_read: manifest.source.record_count,
        records_written: manifest.output.record_count,
    })
}

/// The command line a case is run as, writing a pipeline's configuration out
/// when it needs one.
fn transform_args(
    case: &BenchCase,
    source: &Path,
    output: &Path,
    config: &BenchConfig,
) -> Result<Vec<OsString>, BenchError> {
    let mut args: Vec<OsString> = vec![
        OsString::from("--source"),
        source.into(),
        OsString::from("--output"),
        output.into(),
        OsString::from("--inputs"),
        config.shape.inputs().to_string().into(),
        OsString::from("--outputs"),
        config.shape.outputs().to_string().into(),
        OsString::from("--metadata"),
        format!("bench_case={}", case.label).into(),
    ];

    match &case.transform {
        BenchTransform::Sample { rate } => args.extend([
            OsString::from("sample"),
            OsString::from("--rate"),
            rate.to_string().into(),
        ]),
        BenchTransform::Quantise { scheme } => args.extend([
            OsString::from("quantise"),
            OsString::from("--scheme"),
            scheme.into(),
        ]),
        BenchTransform::Pipeline { config: pipeline } => {
            let path = config
                .workspace
                .join(format!("{}-pipeline.json", case.label));
            fs::write(&path, pipeline.to_json()?).map_err(|e| BenchError::io(&path, e))?;
            args.extend([
                OsString::from("pipeline"),
                OsString::from("--config"),
                path.into(),
            ]);
        }
    }

    Ok(args)
}

/// Measures the Deno sampler over the same corpus, `repeats` times.
///
/// The reference publishes no manifest, so its output size is measured off the
/// directory it published and its counts are read from the JSON summary it
/// prints — the same summary the parity harness reads.
fn measure_reference(
    deno: &DenoReference,
    source: &Path,
    logs: &Path,
    config: &BenchConfig,
    corpus_bytes: u64,
    corpus_records: u64,
) -> Result<Reference, BenchError> {
    let mirrors = config
        .cases
        .iter()
        .find(|case| case.sample_rate().is_some_and(|rate| rate == deno.rate))
        .ok_or_else(|| {
            BenchError::config(format!(
                "the Deno reference samples at {}, and no case in this benchmark samples at that rate",
                deno.rate
            ))
        })?;

    let output = config.workspace.join("out-typescript");
    let mut fastest: Option<RunMeasurement> = None;
    let mut peak_rss_kib: Option<u64> = None;
    let mut peak_rss_method = String::new();
    let mut summary = serde_json::Value::Null;
    for repeat in 1..=config.repeats {
        let command = MeasuredCommand::new(format!("typescript-{repeat}"), "deno", logs)
            .current_dir(&deno.parity_dir)
            .args(["run", "--allow-read", "--allow-write", "grq_sampler.ts"])
            .arg("--source")
            .arg(source)
            .arg("--output")
            .arg(&output)
            .args(["--inputs", &config.shape.inputs().to_string()])
            .args(["--outputs", &config.shape.outputs().to_string()])
            .args(["--rate", &deno.rate.to_string()]);
        let measurement = command.measure()?;
        summary = last_json_line(&command.stdout_path())?;
        peak_rss_kib = match (peak_rss_kib, measurement.peak_rss_kib) {
            (Some(seen), Some(sample)) => Some(seen.max(sample)),
            (seen, sample) => seen.or(sample),
        };
        peak_rss_method.clone_from(&measurement.peak_rss_method);
        if fastest
            .as_ref()
            .is_none_or(|best| measurement.elapsed_ms < best.elapsed_ms)
        {
            fastest = Some(measurement);
        }
    }
    let fastest = fastest
        .ok_or_else(|| BenchError::invariant("repeats", "the reference was not measured at all"))?;

    let records_read = counted(&summary, "recordsRead")?;
    let records_written = counted(&summary, "recordsWritten")?;
    if records_read != corpus_records {
        return Err(BenchError::invariant(
            "corpus coverage",
            format!("the Deno sampler read {records_read} of the {corpus_records} records"),
        ));
    }

    Ok(Reference {
        mirrors: mirrors.label.clone(),
        result: CaseResult {
            label: "typescript".to_string(),
            transform: format!("Sampler.ts --rate {}", deno.rate),
            repeats: config.repeats,
            elapsed_ms: fastest.elapsed_ms,
            peak_rss_kib,
            peak_rss_method,
            input_bytes: corpus_bytes,
            output_bytes: published_bytes(&output)?,
            records_read,
            records_written,
        },
    })
}

/// The bytes the Deno sampler published, summed over the corpus files it wrote.
fn published_bytes(directory: &Path) -> Result<u64, BenchError> {
    let mut total = 0_u64;
    for entry in fs::read_dir(directory).map_err(|e| BenchError::io(directory, e))? {
        let entry = entry.map_err(|e| BenchError::io(directory, e))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("bin") {
            total += entry
                .metadata()
                .map_err(|e| BenchError::io(&path, e))?
                .len();
        }
    }
    if total == 0 {
        return Err(BenchError::invariant(
            "reference output",
            format!("{} holds no published corpus", directory.display()),
        ));
    }
    Ok(total)
}

/// One count off the harness summary, refused rather than defaulted when absent.
fn counted(summary: &serde_json::Value, key: &str) -> Result<u64, BenchError> {
    summary
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            BenchError::invariant(
                "reference summary",
                format!("the Deno sampler did not report {key}: {summary}"),
            )
        })
}

/// The last JSON object the harness printed on standard output.
fn last_json_line(path: &Path) -> Result<serde_json::Value, BenchError> {
    let captured = fs::read_to_string(path).map_err(|e| BenchError::io(path, e))?;
    let line = captured.lines().rev().find(|line| !line.trim().is_empty());
    match line {
        Some(line) => Ok(serde_json::from_str(line)?),
        None => Err(BenchError::invariant(
            "reference summary",
            format!("{} reported nothing on standard output", path.display()),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> BenchConfig {
        BenchConfig {
            workspace: PathBuf::from("/tmp/refinery-bench-args"),
            binary: PathBuf::from("neat_ai_refinery"),
            shape: RecordShape::new(2511, 1).expect("valid shape"),
            shards: 2,
            records_per_shard: 10,
            repeats: 1,
            cases: BenchCase::standard_suite(0.05),
            reference: None,
        }
    }

    fn rendered(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn a_sampling_case_runs_the_production_command_line() {
        let args = transform_args(
            &BenchCase::sample(0.05),
            Path::new("/corpus"),
            Path::new("/out"),
            &config(),
        )
        .expect("build the command line");

        assert_eq!(
            rendered(&args),
            vec![
                "--source",
                "/corpus",
                "--output",
                "/out",
                "--inputs",
                "2511",
                "--outputs",
                "1",
                "--metadata",
                "bench_case=sample",
                "sample",
                "--rate",
                "0.05",
            ]
        );
    }

    #[test]
    fn a_quantisation_case_names_its_scheme() {
        let args = transform_args(
            &BenchCase::quantise(),
            Path::new("/corpus"),
            Path::new("/out"),
            &config(),
        )
        .expect("build the command line");

        let rendered = rendered(&args);
        assert_eq!(
            &rendered[rendered.len() - 3..],
            ["quantise", "--scheme", "bfloat16"]
        );
    }

    #[test]
    fn a_pipeline_case_writes_the_configuration_it_runs() {
        let mut config = config();
        config.workspace = std::env::temp_dir().join(format!(
            "refinery-bench-pipeline-args-{}",
            std::process::id()
        ));
        fs::create_dir_all(&config.workspace).expect("create the workspace");

        let args = transform_args(
            &BenchCase::pipeline(0.05),
            Path::new("/corpus"),
            Path::new("/out"),
            &config,
        )
        .expect("build the command line");

        let rendered = rendered(&args);
        assert_eq!(rendered[rendered.len() - 2], "--config");
        let written = fs::read_to_string(&rendered[rendered.len() - 1])
            .expect("the pipeline configuration is on disk");
        let parsed = crate::pipeline::PipelineConfig::from_json(&written)
            .expect("the configuration the binary would read");
        assert_eq!(parsed.stages.len(), 2);

        fs::remove_dir_all(&config.workspace).expect("clean up");
    }

    #[test]
    fn a_summary_missing_its_counts_fails_loud() {
        let summary = serde_json::json!({ "recordsRead": 10 });

        assert_eq!(counted(&summary, "recordsRead").expect("present"), 10);
        let error =
            counted(&summary, "recordsWritten").expect_err("an absent count is not a zero count");
        assert!(matches!(error, BenchError::Invariant { .. }), "{error:?}");
    }

    #[test]
    fn an_empty_reference_directory_is_not_a_zero_byte_success() {
        let directory = std::env::temp_dir().join(format!(
            "refinery-bench-empty-output-{}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).expect("create the directory");

        let error =
            published_bytes(&directory).expect_err("a sampler that published nothing has failed");

        assert!(matches!(error, BenchError::Invariant { .. }), "{error:?}");
        fs::remove_dir_all(&directory).expect("clean up");
    }
}
