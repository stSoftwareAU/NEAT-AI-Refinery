//! Wiring for the SBOM diff gate (Issue #65).
//!
//! `scripts/test-sbom-diff.sh` exercises the diff itself; these tests hold the
//! configuration around it. A diff step that loses `actions: read` or its call
//! to `sbom-diff.sh` still leaves `sbom.yml` green — it just stops reporting
//! new components, which is exactly the silence the gate exists to prevent.

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

/// The diff step of `sbom.yml`, from its `- name:` line to the next step.
fn diff_step(yaml: &str) -> &str {
    let marker = "- name: Diff SBOM against the Develop baseline";
    let start = yaml
        .find(marker)
        .unwrap_or_else(|| panic!("sbom.yml has no `{marker}` step"));
    let rest = &yaml[start + marker.len()..];
    let end = rest.find("- name:").unwrap_or(rest.len());
    &yaml[start..start + marker.len() + end]
}

#[test]
fn sbom_workflow_diffs_pull_requests_against_the_develop_baseline() {
    let yaml = read(".github/workflows/sbom.yml");
    let step = diff_step(&yaml);
    for needle in [
        "if: github.event_name == 'pull_request'",
        "BASELINE_BRANCH: Develop",
        "--event push --status success",
        "gh run download",
        "./scripts/sbom-diff.sh",
    ] {
        assert!(step.contains(needle), "the SBOM diff step lost `{needle}`");
    }
}

#[test]
fn sbom_workflow_can_read_the_baseline_artefact() {
    let yaml = read(".github/workflows/sbom.yml");
    assert!(
        yaml.contains("actions: read"),
        "sbom.yml needs `actions: read` to download the baseline run's artefact"
    );
}

#[test]
fn sbom_workflow_still_refreshes_the_baseline_on_develop() {
    let yaml = read(".github/workflows/sbom.yml");
    let push = yaml
        .find("\n  push:")
        .expect("sbom.yml must keep its push trigger — it is what writes the baseline");
    let trigger = &yaml[push..yaml.find("\njobs:").expect("sbom.yml has a jobs: block")];
    assert!(
        trigger.contains("Develop"),
        "the push trigger must cover Develop, the branch the diff reads its baseline from"
    );
}

#[test]
fn the_diff_contract_runs_in_ci_and_quality_sh() {
    assert!(
        read(".github/workflows/ci.yml").contains("run: ./scripts/test-sbom-diff.sh"),
        "the shell-checks job must run scripts/test-sbom-diff.sh"
    );
    assert!(
        read("quality.sh").contains("./scripts/test-sbom-diff.sh"),
        "quality.sh must run scripts/test-sbom-diff.sh"
    );
}

#[test]
fn diff_step_stops_at_the_next_step() {
    let yaml = "      - name: Diff SBOM against the Develop baseline\n        run: a\n      - name: Later\n        run: ./scripts/sbom-diff.sh\n";
    assert!(!diff_step(yaml).contains("sbom-diff.sh"));
}

#[test]
fn the_sbom_scripts_are_committed_executable() {
    for script in ["scripts/sbom-diff.sh", "scripts/test-sbom-diff.sh"] {
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
