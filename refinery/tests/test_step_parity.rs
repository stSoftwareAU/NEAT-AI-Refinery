//! The CI test step and the local gate run the same, quiet `cargo test` (Issue #63).
//!
//! `quality.sh` is the local mirror of the `quality` job in `ci.yml`. When the
//! two drift, a PR can pass one gate and not the other. `--verbose` also prints
//! a line for every passing test, so a green run buries the stage that
//! mattered; `cargo test` already prints full failure detail without it.

use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crate directory has a parent")
        .to_path_buf()
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

/// Every `cargo test` invocation in `text`, trimmed and with any YAML `run:`
/// prefix removed, so a workflow step and a shell line compare directly.
fn cargo_test_commands(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .map(|line| line.strip_prefix("run:").map_or(line, str::trim))
        .filter(|line| line.starts_with("cargo test"))
        .map(str::to_string)
        .collect()
}

fn is_verbose(command: &str) -> bool {
    command
        .split_whitespace()
        .take_while(|word| *word != "--")
        .any(|word| matches!(word, "--verbose" | "-v" | "-vv"))
}

#[test]
fn ci_runs_the_same_cargo_test_as_quality_sh() {
    let ci = cargo_test_commands(&read(".github/workflows/ci.yml"));
    let local = cargo_test_commands(&read("quality.sh"));

    assert_eq!(local.len(), 1, "quality.sh runs one cargo test: {local:?}");
    assert_eq!(ci.len(), 1, "ci.yml runs one cargo test: {ci:?}");
    assert_eq!(
        ci, local,
        "the CI test step must match the local gate in quality.sh"
    );
}

#[test]
fn no_gate_runs_cargo_test_verbosely() {
    for file in [".github/workflows/ci.yml", "quality.sh"] {
        let loud: Vec<String> = cargo_test_commands(&read(file))
            .into_iter()
            .filter(|command| is_verbose(command))
            .collect();
        assert!(
            loud.is_empty(),
            "{file} runs cargo test verbosely, printing every passing test: {loud:?}"
        );
    }
}

#[test]
fn verbosity_is_read_from_cargo_flags_only() {
    assert!(is_verbose(
        "cargo test --workspace --verbose -- --test-threads=2"
    ));
    assert!(is_verbose("cargo test -v"));
    assert!(!is_verbose("cargo test --workspace -- --test-threads=2"));
    // A flag after `--` goes to the test binary, not to cargo.
    assert!(!is_verbose("cargo test -- --verbose"));
    assert_eq!(
        cargo_test_commands("      - name: Run tests\n        run: cargo test -q\n"),
        vec!["cargo test -q".to_string()]
    );
    assert!(cargo_test_commands("echo \"Running tests...\"\n").is_empty());
}
