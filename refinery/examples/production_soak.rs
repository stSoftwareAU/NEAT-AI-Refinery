//! Captures the production-soak evidence the GRQ cut-over is gated on.
//!
//! ```bash
//! ./soak/run.sh                       # the whole gate, including the Deno comparison
//! cargo run --release --example production_soak -- --help
//! ```
//!
//! Defaults to the production record shape (2511 inputs, 1 output), eight
//! shards of 20 000 records, a rate of 0.05 and three measured rounds. The
//! corpus is built under the system temporary directory and removed again; the
//! report is written to the evidence directory as
//! `soak-<os>-<arch>.json` and `soak-<os>-<arch>.md`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use neat_ai_refinery::corpus::RecordShape;
use neat_ai_refinery::sample::SampleRate;
use neat_ai_refinery::soak::{soak, DenoReference, SoakConfig};

/// The environment variable GRQ resolves the binary with, reused here so a
/// soak measures the same build the fleet would run.
const BINARY_ENV: &str = "NEAT_AI_REFINERY_BINARY_PATH";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("production_soak: {error}");
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

    let workspace = env::temp_dir().join(format!("refinery-soak-{}", std::process::id()));
    fs::create_dir_all(&workspace)?;

    let config = SoakConfig {
        workspace: workspace.clone(),
        binary: options.binary.clone(),
        shape: RecordShape::new(options.inputs, options.outputs)?,
        shards: options.shards,
        records_per_shard: options.records,
        rate: SampleRate::new(options.rate)?,
        rounds: options.rounds,
        reference: options.reference(),
    };

    println!(
        "Soaking {} — {} shards × {} records at rate {}, {} round(s)",
        config.binary.display(),
        config.shards,
        config.records_per_shard,
        options.rate,
        config.rounds
    );

    // The workspace is scratch either way: a failed soak must not leave a
    // multi-gigabyte corpus behind on the host it just failed on.
    let outcome = soak(&config);
    let _ = fs::remove_dir_all(&workspace);
    let report = outcome?;

    let markdown = report.to_markdown();
    println!("\n{markdown}");

    fs::create_dir_all(&options.evidence)?;
    let stem = options
        .name
        .clone()
        .unwrap_or_else(|| format!("soak-{}-{}", report.host.os, report.host.arch));
    let json_path = options.evidence.join(format!("{stem}.json"));
    let markdown_path = options.evidence.join(format!("{stem}.md"));
    fs::write(&json_path, report.to_json()? + "\n")?;
    fs::write(&markdown_path, &markdown)?;
    println!(
        "Evidence written to {} and {}",
        json_path.display(),
        markdown_path.display()
    );
    Ok(())
}

/// The command-line surface, kept deliberately small.
const USAGE: &str = "\
production_soak — capture Refinery soak evidence for the GRQ cut-over

  --rounds N       measured sampling rounds (default 3)
  --shards N       corpus files to build (default 8)
  --records N      records per corpus file (default 20000)
  --rate R         sampling rate in (0, 1] (default 0.05)
  --inputs N       input values per record (default 2511)
  --outputs N      output values per record (default 1)
  --binary PATH    the neat_ai_refinery binary to soak
                   (default $NEAT_AI_REFINERY_BINARY_PATH, else target/release/neat_ai_refinery)
  --parity DIR     the parity/ directory holding the Deno reference sampler
  --evidence DIR   where the report is written (default docs/evidence)
  --name STEM      file stem for the report (default soak-<os>-<arch>)
  --no-reference   skip the Deno comparison
  --consumer       also run evolve_dir.ts over the published corpus (needs jsr.io)
  --help";

/// The parsed options.
struct Options {
    rounds: usize,
    shards: usize,
    records: usize,
    rate: f64,
    inputs: usize,
    outputs: usize,
    binary: PathBuf,
    parity: PathBuf,
    evidence: PathBuf,
    name: Option<String>,
    no_reference: bool,
    consumer: bool,
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
                "--consumer" => options.consumer = true,
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
        match key {
            "--rounds" => self.rounds = count("--rounds")?,
            "--shards" => self.shards = count("--shards")?,
            "--records" => self.records = count("--records")?,
            "--inputs" => self.inputs = count("--inputs")?,
            "--outputs" => self.outputs = count("--outputs")?,
            "--rate" => {
                self.rate = value
                    .parse()
                    .map_err(|e| format!("--rate must be a number: {e}"))?;
            }
            "--binary" => self.binary = PathBuf::from(value),
            "--parity" => self.parity = PathBuf::from(value),
            "--evidence" => self.evidence = PathBuf::from(value),
            "--name" => self.name = Some(value.to_string()),
            other => return Err(format!("unrecognised option {other}\n\n{USAGE}")),
        }
        Ok(())
    }

    /// The Deno half of the soak, unless it was switched off.
    fn reference(&self) -> Option<DenoReference> {
        if self.no_reference {
            return None;
        }
        Some(DenoReference {
            parity_dir: self.parity.clone(),
            check_consumer: self.consumer,
        })
    }
}

impl Default for Options {
    fn default() -> Self {
        let root = repository_root();
        Self {
            rounds: 3,
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
            no_reference: false,
            consumer: false,
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
