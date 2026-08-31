//! Composing transforms into an ordered pipeline.
//!
//! Every test drives the public [`neat_ai_refinery::pipeline`] API against a
//! real corpus on disk and asserts on the published result, so the checks
//! survive a change of implementation.
//!
//! The four properties under test are the ones a pipeline has to hold:
//! configuration is serialisable and stable, the manifest records the ordered
//! transforms, the same source/config/seed replays byte for byte, and each
//! stage is still the ordinary standalone transform.

// The shared helpers serve several test binaries; not every one uses all of them.
#[allow(dead_code)]
mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{encode, TempDir};
use neat_ai_refinery::corpus::RecordShape;
use neat_ai_refinery::manifest::{CallerMetadata, Manifest, MANIFEST_FILE_NAME};
use neat_ai_refinery::pipeline::{
    run_pipeline, FuzzStage, PipelineConfig, PipelineError, PipelineOutcome, PipelineRequest,
    PipelineStage, QuantiseStage, SampleStage, PIPELINE_CONFIG_VERSION,
};
use neat_ai_refinery::quantise::{quantise, QuantiseRequest, QuantiseScheme};

/// Three inputs and one output — sixteen bytes a record as `f32`.
fn shape() -> RecordShape {
    RecordShape::new(3, 1).expect("valid shape")
}

/// Values spread over several magnitudes, so quantisation and noise both have
/// something to bite on.
fn record_values(index: u32) -> [f32; 4] {
    let step = f64::from(index);
    [
        (0.5 + step * 0.031) as f32,
        (-3.25 + step * 0.017) as f32,
        (1_000.0 + step * 7.5) as f32,
        (0.125 + step * 0.003) as f32,
    ]
}

/// A source directory holding two shards of `count` records each.
fn source_corpus(root: &Path, count: u32) -> PathBuf {
    let source = root.join("trainData-binary");
    fs::create_dir_all(&source).expect("create the source directory");
    for (name, first) in [("shard-a.bin", 0), ("shard-b.bin", count)] {
        let bytes: Vec<u8> = (first..first + count)
            .map(record_values)
            .flat_map(|record| encode(&record))
            .collect();
        fs::write(source.join(name), bytes).expect("write shard");
    }
    source
}

/// A sampling stage that keeps every record, so a pipeline's later stages
/// always have records to work on.
fn sample_stage() -> PipelineStage {
    PipelineStage::Sample(SampleStage {
        rate: 1.0,
        seed: None,
    })
}

/// A gaussian relative-noise stage over the inputs.
fn fuzz_stage() -> PipelineStage {
    PipelineStage::Fuzz(FuzzStage {
        distribution: "gaussian".to_string(),
        scale: 0.01,
        mode: "relative".to_string(),
        targets: "inputs".to_string(),
        clamp_min: None,
        clamp_max: None,
        seed: None,
    })
}

/// A bfloat16 quantisation stage.
fn quantise_stage() -> PipelineStage {
    PipelineStage::Quantise(QuantiseStage {
        scheme: "bfloat16".to_string(),
    })
}

fn config(stages: Vec<PipelineStage>, seed: Option<u64>) -> PipelineConfig {
    let config = PipelineConfig::new(stages);
    match seed {
        Some(seed) => config.with_seed(seed),
        None => config,
    }
}

fn request(source: &Path, output: &Path, config: PipelineConfig) -> PipelineRequest {
    PipelineRequest {
        source: source.to_path_buf(),
        output: output.to_path_buf(),
        shape: shape(),
        config,
        metadata: CallerMetadata::default(),
    }
}

/// Runs `stages` over a fresh corpus under `output`, returning the outcome.
fn run(root: &Path, source: &Path, output_name: &str, config: PipelineConfig) -> PipelineOutcome {
    let output = root.join(output_name);
    run_pipeline(&request(source, &output, config)).expect("the pipeline runs")
}

#[test]
fn round_trips_the_configuration_through_a_stable_json_form() {
    let original = config(
        vec![sample_stage(), fuzz_stage(), quantise_stage()],
        Some(20_260_831),
    );

    let json = original.to_json().expect("the configuration serialises");
    let parsed = PipelineConfig::from_json(&json).expect("the configuration parses back");

    assert_eq!(parsed, original);
    assert_eq!(parsed.version, PIPELINE_CONFIG_VERSION);
    // The wire form is the one an operator writes by hand: the stage is named
    // by `transform`, and the order of `stages` is the order of the run.
    assert!(json.contains("\"transform\": \"sample\""), "{json}");
    assert!(json.contains("\"transform\": \"fuzz\""), "{json}");
    assert!(json.contains("\"transform\": \"quantise\""), "{json}");
    let order: Vec<&str> = ["sample", "fuzz", "quantise"]
        .into_iter()
        .filter(|name| json.contains(&format!("\"transform\": \"{name}\"")))
        .collect();
    assert_eq!(order, vec!["sample", "fuzz", "quantise"]);
}

#[test]
fn loads_a_configuration_an_operator_wrote_by_hand() {
    let temp = TempDir::new("pipeline-config");
    let path = temp.write(
        "pipeline.json",
        br#"{
          "version": 1,
          "seed": 20260831,
          "stages": [
            { "transform": "sample", "rate": 0.5 },
            { "transform": "fuzz", "distribution": "uniform", "scale": 0.02, "mode": "absolute" },
            { "transform": "quantise", "scheme": "bfloat16" }
          ]
        }"#,
    );

    let config = PipelineConfig::load(&path).expect("the configuration loads");

    assert_eq!(config.seed, Some(20_260_831));
    assert_eq!(config.stages.len(), 3);
    assert_eq!(
        config
            .stages
            .iter()
            .map(PipelineStage::name)
            .collect::<Vec<_>>(),
        vec!["sample", "fuzz", "quantise"]
    );
    let PipelineStage::Fuzz(fuzz) = &config.stages[1] else {
        panic!("the second stage is a fuzz stage");
    };
    assert_eq!(
        fuzz.targets, "inputs",
        "an omitted target list defaults to the safe one"
    );
}

#[test]
fn records_the_ordered_transforms_in_the_manifest() {
    let temp = TempDir::new("pipeline-order");
    let source = source_corpus(temp.path(), 16);

    let outcome = run(
        temp.path(),
        &source,
        "derived",
        config(
            vec![sample_stage(), fuzz_stage(), quantise_stage()],
            Some(7),
        ),
    );

    let published =
        Manifest::load(temp.path().join("derived").join(MANIFEST_FILE_NAME)).expect("the manifest");
    assert_eq!(published, outcome.manifest);
    assert_eq!(published.transform.name, "pipeline");
    assert_eq!(published.transform.seed, Some(7));
    let stages = published
        .pipeline
        .as_ref()
        .expect("a pipeline manifest records its stages");
    assert_eq!(
        stages.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        vec!["sample", "fuzz", "quantise"],
        "the manifest records the transforms in the order they ran"
    );
    // Every stage that draws noise records the seed it drew with, so a stage
    // can be replayed on its own.
    assert!(stages[0].seed.is_some(), "sample records its seed");
    assert!(stages[1].seed.is_some(), "fuzz records its seed");
    assert_eq!(stages[2].seed, None, "quantise takes no seed");
    assert_ne!(
        stages[0].seed, stages[1].seed,
        "each stage draws from its own sequence"
    );

    // The published corpus is the last stage's: bfloat16, every record kept.
    assert_eq!(published.record_shape.encoding, "bfloat16");
    assert_eq!(published.record_shape.bytes_per_record, 8);
    assert_eq!(
        published
            .source_record_shape
            .as_ref()
            .expect("the pipeline changed the layout")
            .encoding,
        "float32"
    );
    assert_eq!(published.source.record_count, 32);
    assert_eq!(published.output.record_count, 32);
    assert_eq!(outcome.stages.len(), 3);
    assert_eq!(outcome.records_read, 32);
    assert_eq!(outcome.records_written, 32);

    // Only the published corpus and its manifest survive: the intermediate
    // corpora were scratch and are gone.
    let published_files: Vec<String> = fs::read_dir(temp.path().join("derived"))
        .expect("read the published directory")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(published_files.len(), 2, "{published_files:?}");
}

#[test]
fn replays_the_same_bytes_for_the_same_source_config_and_seed() {
    let temp = TempDir::new("pipeline-replay");
    let source = source_corpus(temp.path(), 16);
    let stages = || vec![sample_stage(), fuzz_stage(), quantise_stage()];

    let first = run(temp.path(), &source, "first", config(stages(), Some(99)));
    let second = run(temp.path(), &source, "second", config(stages(), Some(99)));
    let other = run(temp.path(), &source, "other", config(stages(), Some(100)));

    assert_eq!(
        first.manifest.output.checksum, second.manifest.output.checksum,
        "the same source, config and seed replay byte for byte"
    );
    assert_eq!(
        fs::read(&first.output_file).expect("read the first corpus"),
        fs::read(&second.output_file).expect("read the second corpus")
    );
    assert_ne!(
        first.manifest.output.checksum, other.manifest.output.checksum,
        "a different seed is a different run"
    );
}

#[test]
fn draws_and_reports_a_seed_when_the_configuration_omits_one() {
    let temp = TempDir::new("pipeline-unseeded");
    let source = source_corpus(temp.path(), 8);

    let outcome = run(
        temp.path(),
        &source,
        "derived",
        config(vec![sample_stage(), fuzz_stage()], None),
    );

    assert_eq!(outcome.manifest.transform.seed, Some(outcome.seed));
    let replay = run(
        temp.path(),
        &source,
        "replay",
        config(vec![sample_stage(), fuzz_stage()], Some(outcome.seed)),
    );
    assert_eq!(
        replay.manifest.output.checksum, outcome.manifest.output.checksum,
        "the reported seed is the one that replays the run"
    );
}

#[test]
fn applies_the_stages_in_the_order_they_are_configured() {
    let temp = TempDir::new("pipeline-noncommuting");
    let source = source_corpus(temp.path(), 32);

    let fuzz_then_quantise = run(
        temp.path(),
        &source,
        "fuzz-first",
        config(vec![fuzz_stage(), quantise_stage()], Some(5)),
    );
    let quantise_then_fuzz = run(
        temp.path(),
        &source,
        "quantise-first",
        config(vec![quantise_stage(), fuzz_stage()], Some(5)),
    );

    assert_ne!(
        fs::read(&fuzz_then_quantise.output_file).expect("read the first corpus"),
        fs::read(&quantise_then_fuzz.output_file).expect("read the second corpus"),
        "transforms do not commute, so the configured order decides the output"
    );
    assert_eq!(
        fuzz_then_quantise
            .manifest
            .pipeline
            .as_ref()
            .expect("stages")
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        vec!["fuzz", "quantise"]
    );
    assert_eq!(
        quantise_then_fuzz
            .manifest
            .pipeline
            .as_ref()
            .expect("stages")
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        vec!["quantise", "fuzz"]
    );
}

#[test]
fn is_equivalent_to_running_the_transforms_one_after_another() {
    let temp = TempDir::new("pipeline-equivalence");
    let source = source_corpus(temp.path(), 16);

    let piped = run(
        temp.path(),
        &source,
        "piped",
        config(vec![quantise_stage()], Some(11)),
    );

    // The same transform, run standalone against the same source.
    let standalone = temp.path().join("standalone");
    let outcome = quantise(&QuantiseRequest {
        source: source.clone(),
        output: standalone.clone(),
        shape: shape(),
        scheme: QuantiseScheme::BFloat16,
        metadata: CallerMetadata::default(),
    })
    .expect("the standalone transform runs");

    assert_eq!(
        fs::read(&piped.output_file).expect("read the pipeline corpus"),
        fs::read(&outcome.output_file).expect("read the standalone corpus"),
        "a one-stage pipeline is exactly the standalone transform"
    );
    assert_eq!(
        piped.manifest.output.checksum,
        outcome.manifest.output.checksum
    );
}

#[test]
fn refuses_a_pipeline_with_no_stages() {
    let error = PipelineConfig::new(Vec::new())
        .validate()
        .expect_err("an empty pipeline transforms nothing");

    assert!(matches!(error, PipelineError::EmptyPipeline), "{error:?}");
}

#[test]
fn refuses_a_configuration_version_it_does_not_know() {
    let temp = TempDir::new("pipeline-version");
    let path = temp.write(
        "pipeline.json",
        br#"{ "version": 99, "stages": [{ "transform": "quantise", "scheme": "bfloat16" }] }"#,
    );

    let error = PipelineConfig::load(&path).expect_err("an unknown schema is refused");

    assert!(
        matches!(
            error,
            PipelineError::UnsupportedConfigVersion { found: 99, .. }
        ),
        "{error:?}"
    );
}

#[test]
fn refuses_a_stage_key_it_does_not_know_rather_than_ignoring_it() {
    let temp = TempDir::new("pipeline-typo");
    let path = temp.write(
        "pipeline.json",
        br#"{ "version": 1, "stages": [{ "transform": "sample", "raet": 0.5 }] }"#,
    );

    let error = PipelineConfig::load(&path).expect_err("a misspelt key is refused");

    assert!(matches!(error, PipelineError::Json { .. }), "{error:?}");
}

#[test]
fn refuses_an_unusable_stage_before_a_single_file_is_read() {
    let temp = TempDir::new("pipeline-invalid-stage");
    let source = source_corpus(temp.path(), 8);
    let output = temp.path().join("derived");
    let broken = PipelineConfig::new(vec![
        quantise_stage(),
        PipelineStage::Quantise(QuantiseStage {
            scheme: "int4".to_string(),
        }),
    ]);

    let error = run_pipeline(&request(&source, &output, broken))
        .expect_err("an unknown scheme is fatal before anything is published");

    assert!(
        matches!(error, PipelineError::Stage { position: 2, .. }),
        "{error:?}"
    );
    assert!(
        error.to_string().contains("int4"),
        "the message names the mistake: {error}"
    );
    assert!(
        !output.exists(),
        "nothing is published when a stage cannot run"
    );
}

#[test]
fn publishes_nothing_and_leaves_no_scratch_when_a_stage_fails() {
    let temp = TempDir::new("pipeline-stage-failure");
    let source = source_corpus(temp.path(), 8);
    let output = temp.path().join("derived");
    // bfloat16 → bfloat16 is refused by quantise: the second stage reads a
    // corpus its scheme does not accept.
    let doomed = PipelineConfig::new(vec![quantise_stage(), quantise_stage()]);

    let error =
        run_pipeline(&request(&source, &output, doomed)).expect_err("the second stage fails");

    assert!(
        matches!(error, PipelineError::Stage { position: 2, .. }),
        "{error:?}"
    );
    assert!(!output.exists(), "nothing was published");
    let leftovers: Vec<String> = fs::read_dir(temp.path())
        .expect("read the working directory")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.starts_with('.'))
        .collect();
    assert!(
        leftovers.is_empty(),
        "a failed run leaves no scratch behind: {leftovers:?}"
    );
}
