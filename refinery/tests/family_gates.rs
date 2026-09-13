//! Gates for the family version-increment and `runlib.sh` sync jobs (Issue #53).
//!
//! Both gates are configuration, and configuration is where this pair fails
//! silently. A dropped `paths:` entry leaves source changes shipping under an
//! unchanged crate version, so a fleet host keeps the stale binary it already
//! stamped; a `curl` that has lost `--fail` writes a 404 body over
//! `scripts/runlib.sh` and commits it as the canonical copy. Neither shows up
//! as a red job — they show up as a wrong artefact on a machine nobody is
//! watching. These tests read the committed workflows and fail the build the
//! moment either property goes missing.

use std::fs;
use std::path::{Path, PathBuf};

/// Every path whose change must force a version bump: the crate source, its
/// manifest, the workspace lockfile, and the two files that define the gate
/// itself.
const GATED_PATHS: [&str; 5] = [
    "refinery/src/**",
    "refinery/Cargo.toml",
    "Cargo.lock",
    "scripts/auto-version.sh",
    ".github/workflows/version-increment.yml",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crate directory has a parent")
        .to_path_buf()
}

fn workflow(name: &str) -> String {
    let path = repo_root().join(".github/workflows").join(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
}

/// The body of the job named `job` in `yaml`, up to the next job at the same
/// indentation. Scoping the assertions to one job is the point: a `git rebase`
/// somewhere else in `ci.yml` must not stand in for the one this job needs.
fn job_block(yaml: &str, job: &str) -> String {
    let header = format!("  {job}:");
    let start = yaml
        .find(&header)
        .unwrap_or_else(|| panic!("no `{header}` job in the workflow"));
    let body = &yaml[start + header.len()..];

    let mut block = String::from(&header);
    for line in body.lines() {
        // A job header is exactly two spaces of indent followed by a name.
        let is_next_job = line.starts_with("  ")
            && !line.starts_with("   ")
            && line.trim_end().ends_with(':')
            && !line.trim_start().starts_with('#');
        if is_next_job {
            break;
        }
        block.push('\n');
        block.push_str(line);
    }
    block
}

#[test]
fn version_increment_gates_every_source_path() {
    let yaml = workflow("version-increment.yml");
    for path in GATED_PATHS {
        assert!(
            yaml.contains(&format!("- \"{path}\"")),
            "version-increment.yml does not gate `{path}` — a change there would ship under an \
             unchanged crate version"
        );
    }
}

#[test]
fn version_increment_runs_on_pull_requests_only() {
    let yaml = workflow("version-increment.yml");
    assert!(
        yaml.contains("  pull_request:"),
        "version-increment.yml must run on pull_request"
    );
    assert!(
        !yaml.contains("\n  push:"),
        "version-increment.yml must not run on push — the bump belongs on the PR branch"
    );
}

#[test]
fn version_increment_covers_milestone_branches() {
    let yaml = workflow("version-increment.yml");
    assert!(
        yaml.contains("      - Develop"),
        "version-increment.yml must cover PRs into Develop"
    );
    assert!(
        yaml.contains("      - \"milestone/**\""),
        "version-increment.yml must cover PRs into milestone/** — milestone sub-issue PRs never \
         touch Develop"
    );
}

#[test]
fn version_increment_skips_fork_pull_requests() {
    let block = job_block(&workflow("version-increment.yml"), "version-increment");
    assert!(
        block.contains("github.event.pull_request.head.repo.full_name == github.repository"),
        "the bump job must guard on the PR coming from this repository — it cannot push to a fork"
    );
}

#[test]
fn version_increment_drives_the_bump_script_with_the_lockfile() {
    let block = job_block(&workflow("version-increment.yml"), "version-increment");
    assert!(
        block.contains("./scripts/auto-version.sh refinery/Cargo.toml \"$base\" Cargo.lock"),
        "the bump job must call auto-version.sh with the manifest, the base version and the \
         lockfile, so Cargo.lock does not drift from Cargo.toml"
    );
    assert!(
        block.contains("git rebase \"origin/${HEAD_REF}\""),
        "the bump job must rebase before pushing — family-sync pushes to the same branch"
    );
}

#[test]
fn every_checkout_in_the_family_workflows_refuses_to_persist_credentials() {
    for name in ["version-increment.yml", "ci.yml"] {
        let yaml = workflow(name);
        let checkouts = yaml.matches("uses: actions/checkout@").count();
        let refusals = yaml.matches("persist-credentials: false").count();
        assert_eq!(
            checkouts, refusals,
            "{name} has {checkouts} checkout(s) but {refusals} `persist-credentials: false` — a \
             persisted token is readable by every later step"
        );
    }
}

#[test]
fn family_sync_fetches_the_canonical_copy_and_fails_loud() {
    let block = job_block(&workflow("ci.yml"), "family-sync");
    assert!(
        block.contains(
            "https://raw.githubusercontent.com/stSoftwareAU/NEAT-AI-core/Develop/scripts/runlib.sh"
        ),
        "family-sync must fetch scripts/runlib.sh from NEAT-AI-core Develop — that copy is canonical"
    );
    assert!(
        block.contains("curl --fail"),
        "family-sync must pass --fail to curl: without it a 404 body is written over \
         scripts/runlib.sh and committed as the canonical copy"
    );
    assert!(
        block.contains("cmp -s"),
        "family-sync must compare the fetched copy byte-for-byte"
    );
}

#[test]
fn family_sync_rebases_before_it_pushes_a_refresh() {
    let block = job_block(&workflow("ci.yml"), "family-sync");
    assert!(
        block.contains("git rebase \"origin/${HEAD_REF}\""),
        "family-sync must rebase before pushing — the version-increment workflow pushes to the \
         same branch"
    );
    assert!(
        block.contains("git add scripts/runlib.sh"),
        "family-sync must commit only the synced script"
    );
}

#[test]
fn family_sync_is_part_of_the_aggregated_merge_gate() {
    let yaml = workflow("ci.yml");
    assert!(
        yaml.contains("needs: [validation, quality, security, shell-checks, family-sync]"),
        "ci-required must wait on family-sync, or a failed fetch never blocks the merge"
    );
    let block = job_block(&yaml, "ci-required");
    assert!(
        block.contains("needs.family-sync.result"),
        "ci-required must read the family-sync result"
    );
    assert!(
        block.contains("check \"family-sync\""),
        "ci-required must check the family-sync result, not merely read it"
    );
}

#[test]
fn validation_requires_the_canonical_script_to_be_present() {
    let block = job_block(&workflow("ci.yml"), "validation");
    assert!(
        block.contains("\"scripts/runlib.sh\""),
        "the required-files list must name scripts/runlib.sh"
    );
}

#[test]
fn shell_checks_run_both_script_contracts() {
    let block = job_block(&workflow("ci.yml"), "shell-checks");
    for script in ["./scripts/test-runlib.sh", "./scripts/test-auto-version.sh"] {
        assert!(
            block.contains(script),
            "the shell-checks job must run {script} — a copied script with no contract test is \
             linted but never exercised"
        );
    }
}

#[test]
fn the_family_scripts_are_committed_executable() {
    for script in [
        "scripts/runlib.sh",
        "scripts/auto-version.sh",
        "scripts/test-runlib.sh",
        "scripts/test-auto-version.sh",
    ] {
        let path = repo_root().join(script);
        let metadata = fs::metadata(&path)
            .unwrap_or_else(|error| panic!("cannot stat {}: {error}", path.display()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert!(
                metadata.permissions().mode() & 0o111 != 0,
                "{script} is not executable — CI invokes it directly"
            );
        }
        #[cfg(not(unix))]
        assert!(metadata.is_file(), "{script} is missing");
    }
}

#[test]
fn job_block_stops_at_the_next_job() {
    let yaml = concat!(
        "jobs:\n",
        "  first:\n",
        "    run: alpha\n",
        "  second:\n",
        "    run: beta\n",
    );
    let block = job_block(yaml, "first");
    assert!(block.contains("alpha"), "the named job's body is included");
    assert!(!block.contains("beta"), "the next job's body is excluded");
}
