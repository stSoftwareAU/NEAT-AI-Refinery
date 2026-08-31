//! The command-line surface: `--source`, `--output`, `--inputs`, `--outputs`
//! and the `sample` subcommand.

use std::path::Path;

use clap::Parser;
use neat_ai_refinery::cli::{Cli, Command};
use neat_ai_refinery::sample::SampleError;

/// The invocation shape GRQ will call, as documented in the README.
fn documented_invocation() -> Vec<&'static str> {
    vec![
        "neat_ai_refinery",
        "--source",
        "/data/trainData-binary",
        "--output",
        "/data/trainData-binary-sampler",
        "--inputs",
        "2511",
        "--outputs",
        "1",
        "sample",
        "--rate",
        "0.05",
    ]
}

#[test]
fn parses_the_documented_invocation() {
    let cli = Cli::try_parse_from(documented_invocation()).expect("the documented shape parses");

    assert_eq!(cli.source, Path::new("/data/trainData-binary"));
    assert_eq!(cli.output, Path::new("/data/trainData-binary-sampler"));
    assert_eq!(cli.inputs, 2511);
    assert_eq!(cli.outputs, 1);
    let Command::Sample(args) = &cli.command;
    assert_eq!(args.rate, 0.05);
    assert_eq!(args.seed, None);
}

#[test]
fn builds_a_request_carrying_the_record_shape_and_rate() {
    let cli = Cli::try_parse_from(documented_invocation()).expect("the documented shape parses");

    let request = cli.request().expect("the request is valid");

    assert_eq!(request.shape.inputs(), 2511);
    assert_eq!(request.shape.outputs(), 1);
    assert_eq!(request.shape.bytes_per_record(), 10_048);
    assert_eq!(request.rate.value(), 0.05);
    assert_eq!(request.rate.file_name(), "sample-5.bin");
    assert_eq!(request.seed, None);
}

#[test]
fn accepts_a_seed_for_a_reproducible_run() {
    let mut argv = documented_invocation();
    argv.extend(["--seed", "20260831"]);

    let request = Cli::try_parse_from(argv)
        .expect("a seed is accepted")
        .request()
        .expect("the request is valid");

    assert_eq!(request.seed, Some(20_260_831));
}

#[test]
fn rejects_a_rate_outside_the_allowed_range() {
    let mut argv = documented_invocation();
    argv.pop();
    argv.push("1.5");

    let error = Cli::try_parse_from(argv)
        .expect("clap accepts any float")
        .request()
        .expect_err("the rate is validated");

    assert!(
        matches!(error, SampleError::InvalidRate { .. }),
        "{error:?}"
    );
}

#[test]
fn rejects_a_record_shape_with_no_outputs() {
    let mut argv = documented_invocation();
    let outputs = argv
        .iter()
        .position(|arg| *arg == "--outputs")
        .expect("--outputs is present");
    argv[outputs + 1] = "0";

    let error = Cli::try_parse_from(argv)
        .expect("clap accepts any integer")
        .request()
        .expect_err("a zero-output record shape is invalid");

    assert!(matches!(error, SampleError::Corpus(_)), "{error:?}");
}

#[test]
fn requires_the_rate_and_the_subcommand() {
    let mut without_rate = documented_invocation();
    without_rate.truncate(without_rate.len() - 2);
    assert!(
        Cli::try_parse_from(without_rate).is_err(),
        "--rate is required"
    );

    let mut without_subcommand = documented_invocation();
    without_subcommand.truncate(without_subcommand.len() - 3);
    assert!(
        Cli::try_parse_from(without_subcommand).is_err(),
        "a transform subcommand is required"
    );
}
