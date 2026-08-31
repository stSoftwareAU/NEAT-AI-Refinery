//! Captures the benchmark evidence for issue #14 — throughput, peak RSS and
//! output size, measured rather than claimed.
//!
//! ```bash
//! ./bench/run.sh                        # the whole benchmark, including the Deno comparison
//! cargo run --release --example benchmark -- --help
//! ```
//!
//! Defaults to the production record shape (2511 inputs, 1 output), eight
//! shards of 20 000 records, a rate of 0.05 and three repeats of each case.
//! The corpus is built under the system temporary directory and removed again;
//! the report is written to the evidence directory as `bench-<os>-<arch>.json`
//! and `bench-<os>-<arch>.md`.
//!
//! Two optional gates, both of which fail the run rather than print a warning:
//!
//! ```bash
//! # against a committed report from the same corpus on this host
//! cargo run --release --example benchmark -- --baseline docs/evidence/bench-linux-aarch64.json
//! # against the Deno sampler measured beside it in this very run
//! cargo run --release --example benchmark -- --min-speedup 1.5
//! ```

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use neat_ai_refinery::bench::{
    bench, BenchCase, BenchConfig, BenchReport, Comparison, DenoReference,
};
use neat_ai_refinery::corpus::RecordShape;

/// The environment variable GRQ resolves the binary with, reused here so a
/// benchmark measures the same build the fleet would run.
const BINARY_ENV: &str = "NEAT_AI_REFINERY_BINARY_PATH";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("benchmark: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse(env::args().skip(1))?;
    if options.help {
        println!("{USAGE}");
        return Ok(());
    }

    let workspace = env::temp_dir().join(format!("refinery-bench-{}", std::process::id()));
    fs::create_dir_all(&workspace)?;

    let config = BenchConfig {
        workspace: workspace.clone(),
        binary: options.binary.clone(),
        shape: RecordShape::new(options.inputs, options.outputs)?,
        shards: options.shards,
        records_per_shard: options.records,
        repeats: options.repeats,
        cases: BenchCase::standard_suite(options.rate),
        reference: options.reference(),
    };

    println!(
        "Benchmarking {} — {} shards × {} records at rate {}, {} repeat(s) of {} case(s)",
        config.binary.display(),
        config.shards,
        config.records_per_shard,
        options.rate,
        config.repeats,
        config.cases.len(),
    );

    // The workspace is scratch either way: a failed benchmark must not leave a
    // multi-gigabyte corpus behind on the host it just failed on.
    let outcome = bench(&config);
    let _ = fs::remove_dir_all(&workspace);
    let report = outcome?;

    let markdown = report.to_markdown();
    println!("\n{markdown}");

    fs::create_dir_all(&options.evidence)?;
    let stem = options
        .name
        .clone()
        .unwrap_or_else(|| format!("bench-{}-{}", report.host.os, report.host.arch));
    let json_path = options.evidence.join(format!("{stem}.json"));
    let markdown_path = options.evidence.join(format!("{stem}.md"));
    fs::write(&json_path, report.to_json()? + "\n")?;
    fs::write(&markdown_path, &markdown)?;
    println!(
        "Evidence written to {} and {}",
        json_path.display(),
        markdown_path.display()
    );

    // The gates run last and fail loud: the evidence is on disk either way, so
    // a regression can be read rather than guessed at.
    if let Some(path) = &options.baseline {
        let baseline: BenchReport = serde_json::from_slice(&fs::read(path)?)?;
        let comparison = Comparison::of(&baseline, &report, options.tolerance)?;
        println!("\n{}", comparison.to_markdown());
        comparison.assert_clean()?;
        println!("No regression against {}.", path.display());
    }
    if let Some(minimum) = options.min_speedup {
        let speedup = report.check_speedup(minimum)?;
        println!(
            "Refinery sampled at {speedup:.2}× the Deno sampler, clearing the {minimum:.2}× gate."
        );
    }
    Ok(())
}

/// The command-line surface, kept deliberately small.
const USAGE: &str = "\
benchmark — capture Refinery throughput, peak RSS and output-size evidence

  --repeats N       runs of each case; the fastest is reported (default 3)
  --shards N        corpus files to build (default 8)
  --records N       records per corpus file (default 20000)
  --rate R          sampling rate in (0, 1] (default 0.05)
  --inputs N        input values per record (default 2511)
  --outputs N       output values per record (default 1)
  --binary PATH     the neat_ai_refinery binary to measure
                    (default $NEAT_AI_REFINERY_BINARY_PATH, else target/release/neat_ai_refinery)
  --parity DIR      the parity/ directory holding the Deno reference sampler
  --evidence DIR    where the report is written (default docs/evidence)
  --name STEM       file stem for the report (default bench-<os>-<arch>)
  --baseline FILE   fail if this run regressed against that committed report
  --tolerance F     the drift a baseline comparison allows (default 0.25)
  --min-speedup F   fail unless Refinery beats the Deno sampler by this factor
  --no-reference    skip the Deno comparison
  --help";

/// The parsed options.
struct Options {
    repeats: usize,
    shards: usize,
    records: usize,
    rate: f64,
    inputs: usize,
    outputs: usize,
    binary: PathBuf,
    parity: PathBuf,
    evidence: PathBuf,
    name: Option<String>,
    baseline: Option<PathBuf>,
    tolerance: f64,
    min_speedup: Option<f64>,
    no_reference: bool,
    help: bool,
}

impl Options {
    /// Parses `--key value` pairs, failing loud on anything unrecognised.
    fn parse(argv: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut options = Self::default();
        let mut argv = argv.peekable();
        while let Some(key) = argv.next() {
            match key.as_str() {
                "--help" | "-h" => options.help = true,
                "--no-reference" => options.no_reference = true,
                _ => {
                    let value = argv
                        .next()
                        .ok_or_else(|| format!("{key} needs a value\n\n{USAGE}"))?;
                    options.set(&key, &value)?;
                }
            }
        }
        Ok(options)
    }

    /// Applies one `--key value` pair.
    fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let count = |what: &str| -> Result<usize, String> {
            value
                .parse::<usize>()
                .map_err(|e| format!("{what} must be a whole number: {e}"))
        };
        let number = |what: &str| -> Result<f64, String> {
            value
                .parse::<f64>()
                .map_err(|e| format!("{what} must be a number: {e}"))
        };
        match key {
            "--repeats" => self.repeats = count("--repeats")?,
            "--shards" => self.shards = count("--shards")?,
            "--records" => self.records = count("--records")?,
            "--inputs" => self.inputs = count("--inputs")?,
            "--outputs" => self.outputs = count("--outputs")?,
            "--rate" => self.rate = number("--rate")?,
            "--tolerance" => self.tolerance = number("--tolerance")?,
            "--min-speedup" => self.min_speedup = Some(number("--min-speedup")?),
            "--binary" => self.binary = PathBuf::from(value),
            "--parity" => self.parity = PathBuf::from(value),
            "--evidence" => self.evidence = PathBuf::from(value),
            "--baseline" => self.baseline = Some(PathBuf::from(value)),
            "--name" => self.name = Some(value.to_string()),
            other => return Err(format!("unrecognised option {other}\n\n{USAGE}")),
        }
        Ok(())
    }

    /// The Deno comparison, unless it was switched off.
    fn reference(&self) -> Option<DenoReference> {
        if self.no_reference {
            return None;
        }
        Some(DenoReference {
            parity_dir: self.parity.clone(),
            rate: self.rate,
        })
    }
}

impl Default for Options {
    fn default() -> Self {
        let root = repository_root();
        Self {
            repeats: 3,
            shards: 8,
            records: 20_000,
            rate: 0.05,
            inputs: 2511,
            outputs: 1,
            binary: env::var_os(BINARY_ENV).map_or_else(
                || root.join("target/release/neat_ai_refinery"),
                PathBuf::from,
            ),
            parity: root.join("parity"),
            evidence: root.join("docs/evidence"),
            name: None,
            baseline: None,
            tolerance: 0.25,
            min_speedup: None,
            no_reference: false,
            help: false,
        }
    }
}

/// The workspace root this example was built from.
fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf()
}
