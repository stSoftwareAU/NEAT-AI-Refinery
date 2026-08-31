//! The command-line surface: `--source`, `--output`, `--inputs`, `--outputs`
//! and the `sample` subcommand.

use std::path::Path;

use clap::Parser;
use neat_ai_refinery::cli::{Cli, CliError, Command, TransformRequest};
use neat_ai_refinery::fuzz::{FuzzDistribution, FuzzError, FuzzMode, FuzzRequest, FuzzTargets};
use neat_ai_refinery::pipeline::{PipelineError, PipelineStage};
use neat_ai_refinery::quantise::{QuantiseError, QuantiseScheme};
use neat_ai_refinery::sample::{SampleError, SampleRequest};

/// The sampling request behind a parsed command line.
fn sample_request(cli: &Cli) -> Result<SampleRequest, CliError> {
    match cli.request()? {
        TransformRequest::Sample(request) => Ok(*request),
        other => panic!("expected a sample request, got {other:?}"),
    }
}

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
    let Command::Sample(args) = &cli.command else {
        panic!("the documented invocation is a sample run");
    };
    assert_eq!(args.rate, 0.05);
    assert_eq!(args.seed, None);
}

#[test]
fn builds_a_request_carrying_the_record_shape_and_rate() {
    let cli = Cli::try_parse_from(documented_invocation()).expect("the documented shape parses");

    let request = sample_request(&cli).expect("the request is valid");

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

    let cli = Cli::try_parse_from(argv).expect("a seed is accepted");
    let request = sample_request(&cli).expect("the request is valid");

    assert_eq!(request.seed, Some(20_260_831));
}

#[test]
fn carries_repeated_caller_metadata_into_the_request() {
    let mut argv = documented_invocation();
    argv.splice(
        9..9,
        [
            "--metadata",
            "grq_observation_version=42",
            "--metadata",
            "run.label=nightly",
        ],
    );

    let cli = Cli::try_parse_from(argv).expect("repeated --metadata is accepted");
    let request = sample_request(&cli).expect("the request is valid");

    assert_eq!(request.metadata.get("grq_observation_version"), Some("42"));
    assert_eq!(request.metadata.get("run.label"), Some("nightly"));
    assert_eq!(request.metadata.len(), 2);
}

#[test]
fn rejects_caller_metadata_that_is_not_a_key_value_pair() {
    let mut argv = documented_invocation();
    argv.splice(9..9, ["--metadata", "no-equals-sign"]);

    let error = Cli::try_parse_from(argv)
        .expect("clap accepts any string")
        .request()
        .expect_err("the metadata is validated");

    assert!(
        matches!(error, CliError::Sample(SampleError::Manifest(_))),
        "{error:?}"
    );
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
        matches!(error, CliError::Sample(SampleError::InvalidRate { .. })),
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

    assert!(
        matches!(error, CliError::Sample(SampleError::Corpus(_))),
        "{error:?}"
    );
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

/// The `quantise` invocation documented in the README, over a sampled corpus.
fn documented_quantise_invocation() -> Vec<&'static str> {
    vec![
        "neat_ai_refinery",
        "--source",
        "/data/trainData-binary-sampler",
        "--output",
        "/data/trainData-binary-sampler-bf16",
        "--inputs",
        "2511",
        "--outputs",
        "1",
        "quantise",
        "--scheme",
        "bfloat16",
    ]
}

#[test]
fn parses_the_documented_quantise_invocation() {
    let cli =
        Cli::try_parse_from(documented_quantise_invocation()).expect("the documented shape parses");

    let Command::Quantise(args) = &cli.command else {
        panic!("the documented invocation is a quantise run");
    };
    assert_eq!(args.scheme, "bfloat16");

    let TransformRequest::Quantise(request) = cli.request().expect("the request is valid") else {
        panic!("expected a quantise request");
    };
    assert_eq!(request.scheme, QuantiseScheme::BFloat16);
    // The caller states the *source* shape; the published width follows from
    // the scheme rather than from another flag.
    assert_eq!(request.shape.bytes_per_record(), 10_048);
    assert_eq!(
        request
            .target_shape()
            .expect("the narrower shape exists")
            .bytes_per_record(),
        5_024
    );
}

#[test]
fn rejects_a_quantisation_scheme_refinery_does_not_offer() {
    let mut argv = documented_quantise_invocation();
    argv.pop();
    argv.push("int4");

    let error = Cli::try_parse_from(argv)
        .expect("clap accepts any string")
        .request()
        .expect_err("the scheme is validated");

    assert!(
        matches!(
            error,
            CliError::Quantise(QuantiseError::UnknownScheme { .. })
        ),
        "{error:?}"
    );
}

#[test]
fn requires_a_scheme_rather_than_defaulting_to_one() {
    let mut without_scheme = documented_quantise_invocation();
    without_scheme.truncate(without_scheme.len() - 2);

    assert!(
        Cli::try_parse_from(without_scheme).is_err(),
        "--scheme is required: the scheme decides the error the corpus carries"
    );
}

#[test]
fn carries_caller_metadata_into_a_quantise_request() {
    let mut argv = documented_quantise_invocation();
    argv.splice(9..9, ["--metadata", "grq_observation_version=42"]);

    let TransformRequest::Quantise(request) = Cli::try_parse_from(argv)
        .expect("--metadata is accepted")
        .request()
        .expect("the request is valid")
    else {
        panic!("expected a quantise request");
    };

    assert_eq!(request.metadata.get("grq_observation_version"), Some("42"));
}

/// The `fuzz` invocation documented in the README.
fn documented_fuzz_invocation() -> Vec<&'static str> {
    vec![
        "neat_ai_refinery",
        "--source",
        "/data/trainData-binary",
        "--output",
        "/data/trainData-binary-fuzzed",
        "--inputs",
        "2511",
        "--outputs",
        "1",
        "fuzz",
        "--distribution",
        "gaussian",
        "--scale",
        "0.01",
        "--mode",
        "relative",
    ]
}

/// The fuzzing request behind a parsed command line.
fn fuzz_request(argv: Vec<&str>) -> Result<FuzzRequest, CliError> {
    match Cli::try_parse_from(argv)
        .expect("clap accepts the arguments")
        .request()?
    {
        TransformRequest::Fuzz(request) => Ok(*request),
        other => panic!("expected a fuzz request, got {other:?}"),
    }
}

#[test]
fn parses_the_documented_fuzz_invocation() {
    let cli =
        Cli::try_parse_from(documented_fuzz_invocation()).expect("the documented shape parses");

    let Command::Fuzz(args) = &cli.command else {
        panic!("the documented invocation is a fuzz run");
    };
    assert_eq!(args.distribution, "gaussian");
    assert_eq!(args.scale, 0.01);
    assert_eq!(args.mode, "relative");
    assert_eq!(args.clamp_min, None);
    assert_eq!(args.clamp_max, None);
    assert_eq!(args.seed, None);

    let request = fuzz_request(documented_fuzz_invocation()).expect("the request is valid");
    assert_eq!(request.policy.distribution(), FuzzDistribution::Gaussian);
    assert_eq!(request.policy.scale(), 0.01);
    assert_eq!(request.policy.mode(), FuzzMode::Relative);
    // The record layout is unchanged by fuzzing, so there is one shape.
    assert_eq!(request.shape.bytes_per_record(), 10_048);
}

#[test]
fn targets_inputs_unless_the_caller_asks_for_more() {
    let request = fuzz_request(documented_fuzz_invocation()).expect("the request is valid");

    assert_eq!(
        request.policy.targets(),
        FuzzTargets::Inputs,
        "an expected output is never perturbed by default"
    );

    let mut argv = documented_fuzz_invocation();
    argv.extend(["--targets", "all"]);
    let explicit = fuzz_request(argv).expect("the request is valid");

    assert_eq!(explicit.policy.targets(), FuzzTargets::All);
}

#[test]
fn carries_the_bounds_and_the_seed_into_the_policy() {
    let mut argv = documented_fuzz_invocation();
    argv.extend([
        "--clamp-min",
        "-1",
        "--clamp-max",
        "1",
        "--seed",
        "20260831",
    ]);

    let request = fuzz_request(argv).expect("the request is valid");

    assert_eq!(request.policy.bounds().min(), Some(-1.0));
    assert_eq!(request.policy.bounds().max(), Some(1.0));
    assert_eq!(request.seed, Some(20_260_831));
}

#[test]
fn rejects_a_distribution_mode_or_target_refinery_does_not_offer() {
    let replace = |flag: &'static str, value: &'static str| {
        let mut argv = documented_fuzz_invocation();
        match argv.iter().position(|arg| *arg == flag) {
            Some(index) => argv[index + 1] = value,
            None => argv.extend([flag, value]),
        }
        argv
    };

    assert!(matches!(
        fuzz_request(replace("--distribution", "cauchy")).expect_err("validated"),
        CliError::Fuzz(FuzzError::UnknownDistribution { .. })
    ));
    assert!(matches!(
        fuzz_request(replace("--mode", "multiplicative")).expect_err("validated"),
        CliError::Fuzz(FuzzError::UnknownMode { .. })
    ));
    assert!(matches!(
        fuzz_request(replace("--targets", "everything")).expect_err("validated"),
        CliError::Fuzz(FuzzError::UnknownTargets { .. })
    ));
}

#[test]
fn rejects_a_scale_that_is_not_a_positive_finite_number() {
    for scale in ["0", "-1", "nan", "inf"] {
        let mut argv = documented_fuzz_invocation();
        let index = argv
            .iter()
            .position(|arg| *arg == "--scale")
            .expect("--scale is present");
        argv[index + 1] = scale;

        let error = fuzz_request(argv).expect_err("the scale is validated");

        assert!(
            matches!(error, CliError::Fuzz(FuzzError::InvalidScale { .. })),
            "{scale}: {error:?}"
        );
    }
}

#[test]
fn rejects_bounds_that_cross() {
    let mut argv = documented_fuzz_invocation();
    argv.extend(["--clamp-min", "1", "--clamp-max", "-1"]);

    let error = fuzz_request(argv).expect_err("the bounds are validated");

    assert!(
        matches!(error, CliError::Fuzz(FuzzError::InvalidBounds { .. })),
        "{error:?}"
    );
}

#[test]
fn requires_the_distribution_the_scale_and_the_mode() {
    for flag in ["--distribution", "--scale", "--mode"] {
        let mut argv = documented_fuzz_invocation();
        let index = argv
            .iter()
            .position(|arg| *arg == flag)
            .expect("the flag is present");
        argv.drain(index..index + 2);

        assert!(
            Cli::try_parse_from(argv).is_err(),
            "{flag} states part of the perturbation, so it is never defaulted"
        );
    }
}

#[test]
fn carries_caller_metadata_into_a_fuzz_request() {
    let mut argv = documented_fuzz_invocation();
    argv.splice(9..9, ["--metadata", "grq_observation_version=42"]);

    let request = fuzz_request(argv).expect("the request is valid");

    assert_eq!(request.metadata.get("grq_observation_version"), Some("42"));
}

/// The documented `pipeline` invocation, reading the configuration from `path`.
fn documented_pipeline_invocation(path: &str) -> Vec<String> {
    [
        "neat_ai_refinery",
        "--source",
        "/data/trainData-binary",
        "--output",
        "/data/trainData-binary-refined",
        "--inputs",
        "2511",
        "--outputs",
        "1",
        "pipeline",
        "--config",
        path,
    ]
    .iter()
    .map(|arg| (*arg).to_string())
    .collect()
}

/// Writes a pipeline configuration under a unique temporary path.
fn write_pipeline_config(label: &str, json: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "neat-ai-refinery-cli-{label}-{}-{}.json",
        std::process::id(),
        line!()
    ));
    std::fs::write(&path, json).expect("write the configuration");
    path
}

#[test]
fn builds_a_pipeline_request_carrying_the_ordered_stages() {
    let path = write_pipeline_config(
        "ordered",
        r#"{
          "version": 1,
          "seed": 20260831,
          "stages": [
            { "transform": "sample", "rate": 0.05 },
            { "transform": "fuzz", "distribution": "gaussian", "scale": 0.01, "mode": "relative" },
            { "transform": "quantise", "scheme": "bfloat16" }
          ]
        }"#,
    );
    let cli = Cli::try_parse_from(documented_pipeline_invocation(&path.to_string_lossy()))
        .expect("the documented shape parses");

    let TransformRequest::Pipeline(request) = cli.request().expect("the request is valid") else {
        panic!("a pipeline invocation builds a pipeline request");
    };

    assert_eq!(request.shape.inputs(), 2511);
    assert_eq!(request.config.seed, Some(20_260_831));
    assert_eq!(
        request
            .config
            .stages
            .iter()
            .map(PipelineStage::name)
            .collect::<Vec<_>>(),
        vec!["sample", "fuzz", "quantise"],
        "the request preserves the configured order"
    );
    std::fs::remove_file(&path).expect("remove the configuration");
}

#[test]
fn rejects_a_pipeline_configuration_it_cannot_read() {
    let cli = Cli::try_parse_from(documented_pipeline_invocation("/does/not/exist.json"))
        .expect("the shape parses");

    let error = cli.request().expect_err("a missing configuration is fatal");

    assert!(
        matches!(error, CliError::Pipeline(PipelineError::Io { .. })),
        "{error:?}"
    );
}

#[test]
fn rejects_a_pipeline_with_a_stage_it_cannot_run() {
    let path = write_pipeline_config(
        "unusable",
        r#"{ "version": 1, "stages": [{ "transform": "quantise", "scheme": "int4" }] }"#,
    );
    let cli = Cli::try_parse_from(documented_pipeline_invocation(&path.to_string_lossy()))
        .expect("the shape parses");

    let error = cli
        .request()
        .expect_err("an unknown scheme is refused at parse time");

    assert!(
        matches!(
            error,
            CliError::Pipeline(PipelineError::Stage { position: 1, .. })
        ),
        "{error:?}"
    );
    std::fs::remove_file(&path).expect("remove the configuration");
}

#[test]
fn requires_a_configuration_for_a_pipeline_run() {
    let mut argv = documented_pipeline_invocation("unused.json");
    argv.truncate(argv.len() - 2);

    assert!(
        Cli::try_parse_from(argv).is_err(),
        "a pipeline states its stages, so the configuration is never defaulted"
    );
}
