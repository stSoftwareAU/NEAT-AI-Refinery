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

/// The same-repo guard belongs on the push, not on the job: a fork PR cannot
/// receive a pushed commit, but the checks that need no write access — the
/// downgrade check, and the byte-for-byte comparison against core — must still
/// run for it. Guarding the whole job turned both into a skip, and `ci-required`
/// scores a skip as OK.
#[test]
fn only_the_push_is_guarded_to_this_repository() {
    let guard = "github.event.pull_request.head.repo.full_name == github.repository";

    let bump = job_block(&workflow("version-increment.yml"), "version-increment");
    let (bump_header, bump_steps) = bump
        .split_once("    steps:")
        .expect("the bump job declares steps");
    assert!(
        !bump_header.contains(guard),
        "the bump job must not be skipped wholesale on a fork — the downgrade check needs no \
         write access"
    );
    assert!(
        bump_steps.contains(&format!("        if: {guard}")),
        "the bump's push step must be guarded to this repository"
    );

    let sync = job_block(&workflow("ci.yml"), "family-sync");
    let (sync_header, sync_steps) = sync
        .split_once("    steps:")
        .expect("the family-sync job declares steps");
    assert!(
        !sync_header.contains(guard),
        "family-sync must not be skipped wholesale on a fork — the comparison needs no write access"
    );
    assert!(
        sync_steps.contains(guard),
        "family-sync's push step must be guarded to this repository"
    );
}

/// Drift is a failure even once it has been corrected. The refreshed copy is
/// pushed rather than gated — `shell-checks` ran against the bytes the PR
/// arrived with, and a commit pushed with the default token starts no run of
/// its own — so a green job here would report unlinted content as checked.
#[test]
fn family_sync_fails_the_run_when_the_copy_had_drifted() {
    let block = job_block(&workflow("ci.yml"), "family-sync");
    let (_, after_compare) = block
        .split_once("Fail the run when the copy had drifted")
        .expect("family-sync must carry a step that fails on drift");
    assert!(
        after_compare.contains("if: steps.compare.outputs.drifted == 'true'"),
        "the failing step must be conditioned on drift"
    );
    assert!(
        after_compare.contains("exit 1"),
        "the failing step must exit non-zero, or drift passes silently"
    );
}

/// The runner masks the secret, not its base64, so an encoded credential that
/// is never `::add-mask::`-ed survives any later echo or `set -x` in the log.
/// `base64 -w0` is GNU-only besides, and this repository runs macOS jobs.
#[test]
fn the_push_credential_is_masked_and_encoded_portably() {
    for (name, job) in [
        ("ci.yml", "family-sync"),
        ("version-increment.yml", "version-increment"),
    ] {
        let block = job_block(&workflow(name), job);
        assert!(
            block.contains("echo \"::add-mask::${credential}\""),
            "{name}:{job} must mask the encoded credential"
        );
        assert!(
            block.contains("base64 | tr -d"),
            "{name}:{job} must encode with `base64 | tr -d`"
        );
        // The comment above the command names `base64 -w0` to explain why it
        // is not used, so only executable lines are searched for it.
        let gnu_only = block
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .any(|line| line.contains("base64 -w0"));
        assert!(
            !gnu_only,
            "{name}:{job} runs `base64 -w0`, which is GNU-only — this repository runs macOS jobs"
        );
    }
}

/// The bump rewrites `Cargo.lock` with awk and nothing downstream reads the
/// lockfile before the commit merges, so the workflow confirms the two agree.
#[test]
fn the_bump_verifies_the_lockfile_against_the_manifest() {
    let block = job_block(&workflow("version-increment.yml"), "version-increment");
    assert!(
        block.contains("if [ \"$manifest_version\" != \"$lock_version\" ]; then"),
        "the bump must compare the rewritten Cargo.lock against refinery/Cargo.toml"
    );
}

/// On a fork PR `origin` is the fork, whose copy of the base branch may be
/// stale or missing entirely — comparing against that is worse than not
/// comparing at all.
#[test]
fn the_base_version_is_read_from_this_repository() {
    let block = job_block(&workflow("version-increment.yml"), "version-increment");
    assert!(
        block.contains("git fetch --no-tags \"https://github.com/${GITHUB_REPOSITORY}.git\""),
        "the base branch must be fetched from this repository, not from the PR's origin"
    );
    assert!(
        block.contains("base/${BASE_REF}:refinery/Cargo.toml"),
        "the base manifest must be read from the ref fetched above"
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
