//! Drift gate for the shared corpus-runner workflow (Issue #68).
//!
//! `benchmark.yml`, `parity.yml` and `soak.yml` each copy-pasted the same
//! checkout, Rust setup, Deno setup, corpus script and evidence upload. A
//! change to any of those steps had to be made three times by hand, and one
//! that reached only two of the three reintroduced the drift.
//!
//! The block now lives once, in `.github/workflows/_corpus-runner.yml`, and
//! each caller passes only what differs: its script, its arguments, its cache
//! suffix and its evidence name. These tests read the real workflow files and
//! hold that line.

mod support;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use support::workflow_yaml::invocation_inputs;

const RUNNER: &str = "_corpus-runner.yml";
const RUNNER_USES: &str = "./.github/workflows/_corpus-runner.yml";

/// The steps the runner owns, which no caller may inline again.
const SHARED_STEPS: [&str; 4] = [
    "actions/checkout",
    "./.github/actions/rust-setup",
    "denoland/setup-deno",
    "actions/upload-artifact",
];

/// What each caller passes to the runner. An absent input reads as `""`.
struct Caller {
    file: &'static str,
    cache_key_suffix: &'static str,
    script: &'static str,
    args: &'static str,
    evidence_name: &'static str,
    job_summary: &'static str,
}

const CALLERS: [Caller; 3] = [
    Caller {
        file: "benchmark.yml",
        cache_key_suffix: "bench",
        script: "./bench/run.sh",
        args: "--shards 4 --records 10000 --repeats 2 --min-speedup 1.25",
        evidence_name: "bench",
        job_summary: "true",
    },
    Caller {
        file: "parity.yml",
        cache_key_suffix: "parity",
        script: "./parity/run.sh",
        args: "",
        evidence_name: "",
        job_summary: "",
    },
    Caller {
        file: "soak.yml",
        cache_key_suffix: "soak",
        script: "./soak/run.sh",
        args: "--shards 2 --records 5000 --rounds 3",
        evidence_name: "soak",
        job_summary: "",
    },
];

fn workflow(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crate directory has a parent")
        .join(".github/workflows")
        .join(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

fn input<'a>(inputs: &'a BTreeMap<String, String>, key: &str) -> &'a str {
    inputs.get(key).map(String::as_str).unwrap_or("")
}

/// The top-level `on:` triggers of `yaml` — the keys indented one level
/// beneath `on:`.
fn triggers(yaml: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut in_on = false;
    for line in yaml.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        if indent == 0 {
            in_on = trimmed == "on:";
            continue;
        }
        if in_on && indent == 2 {
            if let Some((key, _)) = trimmed.split_once(':') {
                found.push(key.to_string());
            }
        }
    }
    found
}

#[test]
fn each_caller_invokes_the_runner_once_with_its_own_inputs() {
    for caller in CALLERS {
        let yaml = workflow(caller.file);
        let invocations = invocation_inputs(&yaml, RUNNER_USES);
        assert_eq!(
            invocations.len(),
            1,
            "{} should call `{RUNNER_USES}` exactly once",
            caller.file
        );
        let inputs = &invocations[0];
        for (key, expected) in [
            ("cache-key-suffix", caller.cache_key_suffix),
            ("script", caller.script),
            ("args", caller.args),
            ("evidence-name", caller.evidence_name),
            ("job-summary", caller.job_summary),
        ] {
            assert_eq!(
                input(inputs, key),
                expected,
                "{} passes the wrong `{key}` to the corpus runner",
                caller.file
            );
        }
    }
}

#[test]
fn no_caller_inlines_the_shared_steps() {
    let mut offenders = Vec::new();
    for caller in CALLERS {
        let yaml = workflow(caller.file);
        for step in SHARED_STEPS {
            if !invocation_inputs(&yaml, step).is_empty() {
                offenders.push(format!("{} inlines `{step}`", caller.file));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the shared steps belong to `{RUNNER}` alone:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_runner_owns_each_shared_step_exactly_once() {
    let yaml = workflow(RUNNER);
    for step in SHARED_STEPS {
        assert_eq!(
            invocation_inputs(&yaml, step).len(),
            1,
            "{RUNNER} should run `{step}` exactly once"
        );
    }
}

#[test]
fn the_runner_checkout_does_not_persist_the_token() {
    let yaml = workflow(RUNNER);
    for inputs in invocation_inputs(&yaml, "actions/checkout") {
        assert_eq!(
            input(&inputs, "persist-credentials"),
            "false",
            "no runner step pushes back, so the GITHUB_TOKEN must never reach .git/config"
        );
    }
}

#[test]
fn the_runner_forwards_the_cache_key_suffix() {
    let yaml = workflow(RUNNER);
    let setup = invocation_inputs(&yaml, "./.github/actions/rust-setup");
    assert_eq!(
        input(&setup[0], "cache-key-suffix"),
        "${{ inputs.cache-key-suffix }}",
        "each caller's cache must stay distinct, so the suffix has to reach rust-setup"
    );
}

#[test]
fn the_runner_uploads_the_named_evidence_or_fails() {
    let yaml = workflow(RUNNER);
    let upload = invocation_inputs(&yaml, "actions/upload-artifact");
    let upload = &upload[0];
    assert!(
        input(upload, "name").contains("inputs.evidence-name"),
        "the artifact name must come from the caller's evidence name"
    );
    assert!(
        input(upload, "path").contains("inputs.evidence-name"),
        "only the caller's own evidence directory may be uploaded"
    );
    assert_eq!(
        input(upload, "if-no-files-found"),
        "error",
        "a run that wrote no evidence must fail rather than upload nothing"
    );
}

#[test]
fn the_runner_is_only_reachable_through_workflow_call() {
    assert_eq!(
        triggers(&workflow(RUNNER)),
        vec!["workflow_call".to_string()],
        "{RUNNER} is a building block — its callers own the triggers"
    );
}

#[test]
fn triggers_reads_only_the_on_block() {
    let yaml = concat!(
        "name: Example\n",
        "on:\n",
        "  # a comment\n",
        "  pull_request:\n",
        "    branches: [\"**\"]\n",
        "  workflow_dispatch:\n",
        "permissions:\n",
        "  contents: read\n",
    );
    assert_eq!(triggers(yaml), vec!["pull_request", "workflow_dispatch"]);
    assert_eq!(triggers(""), Vec::<String>::new());
}
