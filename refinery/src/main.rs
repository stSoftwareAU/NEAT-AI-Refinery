//! The `neat_ai_refinery` binary: parse the arguments, run the transform, and
//! fail loud with a non-zero exit when it cannot be completed.

use std::process::ExitCode;

use clap::Parser;
use neat_ai_refinery::cli::{Cli, CliError, TransformRequest};
use neat_ai_refinery::fuzz::{fuzz, FuzzOutcome};
use neat_ai_refinery::manifest::Manifest;
use neat_ai_refinery::quantise::{quantise, QuantiseOutcome};
use neat_ai_refinery::sample::{sample, SampleOutcome};

fn main() -> ExitCode {
    let cli = Cli::parse();

    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neat_ai_refinery: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Runs the requested transform and reports what it published.
fn run(cli: &Cli) -> Result<(), CliError> {
    match cli.request()? {
        TransformRequest::Sample(request) => report_sample(&sample(&request)?),
        TransformRequest::Quantise(request) => report_quantise(&quantise(&request)?),
        TransformRequest::Fuzz(request) => report_fuzz(&fuzz(&request)?),
    }
    Ok(())
}

/// Reports a sampling run, including the seed needed to reproduce it.
fn report_sample(outcome: &SampleOutcome) {
    println!(
        "🏭 {} — {} of {} records kept from {} file(s), seed {}",
        outcome.output_file.display(),
        outcome.records_written,
        outcome.records_read,
        outcome.sources.len(),
        outcome.seed
    );
    report_manifest(&outcome.manifest_file, &outcome.manifest);
}

/// Reports a quantisation run, including the storage it saved.
fn report_quantise(outcome: &QuantiseOutcome) {
    println!(
        "🏭 {} — {} records re-encoded as {} from {} file(s), {:.1}% smaller ({} → {} bytes)",
        outcome.output_file.display(),
        outcome.records_written,
        outcome.manifest.record_shape.encoding,
        outcome.sources.len(),
        outcome.storage_reduction() * 100.0,
        outcome.source_bytes,
        outcome.output_bytes
    );
    report_manifest(&outcome.manifest_file, &outcome.manifest);
}

/// Reports a fuzzing run, including the seed needed to reproduce it and every
/// value the policy could not simply perturb.
fn report_fuzz(outcome: &FuzzOutcome) {
    println!(
        "🏭 {} — {} records, {} values perturbed ({} clamped, {} non-finite preserved) from {} file(s), seed {}",
        outcome.output_file.display(),
        outcome.records_written,
        outcome.values_perturbed,
        outcome.values_clamped,
        outcome.values_preserved,
        outcome.sources.len(),
        outcome.seed
    );
    report_manifest(&outcome.manifest_file, &outcome.manifest);
}

/// Reports the provenance published beside the corpus.
fn report_manifest(path: &std::path::Path, manifest: &Manifest) {
    println!(
        "📄 {} — {} {}",
        path.display(),
        manifest.output.checksum.algorithm,
        manifest.output.checksum.value
    );
}
