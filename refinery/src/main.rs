//! The `neat_ai_refinery` binary: parse the arguments, run the transform, and
//! fail loud with a non-zero exit when it cannot be completed.

use std::process::ExitCode;

use clap::Parser;
use neat_ai_refinery::cli::Cli;
use neat_ai_refinery::sample::{sample, SampleError, SampleOutcome};

fn main() -> ExitCode {
    let cli = Cli::parse();

    match run(&cli) {
        Ok(outcome) => {
            report(&outcome);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("neat_ai_refinery: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Runs the requested transform.
fn run(cli: &Cli) -> Result<SampleOutcome, SampleError> {
    sample(&cli.request()?)
}

/// Reports what was published, including the seed needed to reproduce it.
fn report(outcome: &SampleOutcome) {
    println!(
        "🏭 {} — {} of {} records kept from {} file(s), seed {}",
        outcome.output_file.display(),
        outcome.records_written,
        outcome.records_read,
        outcome.sources.len(),
        outcome.seed
    );
}
