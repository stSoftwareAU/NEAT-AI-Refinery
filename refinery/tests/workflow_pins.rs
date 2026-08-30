//! Supply-chain gate for the repository's GitHub Actions workflows.
//!
//! Every third-party `uses:` reference must be pinned to a 40-character commit
//! SHA with a trailing `# <version>` comment, and every `image:` reference must
//! be pinned by `@sha256:` digest. Tags are mutable — a re-tagged or
//! compromised upstream would otherwise silently re-execute with this
//! repository's workflow privileges.
//!
//! The checks run as ordinary tests so `cargo test` fails the moment a
//! workflow gains an unpinned reference.

use std::fs;
use std::path::{Path, PathBuf};

/// A workflow reference that breaches the pinning policy.
#[derive(Debug, PartialEq, Eq)]
struct Violation {
    line: usize,
    reason: String,
    text: String,
}

/// Returns every `uses:` reference in `yaml` that is not SHA-pinned with a
/// trailing `# <version>` comment. Local actions and reusable workflows
/// (`uses: ./…`) are exempt — they are versioned by this repository itself.
fn unpinned_uses(yaml: &str) -> Vec<Violation> {
    let mut violations = Vec::new();

    for (index, raw_line) in yaml.lines().enumerate() {
        let line = raw_line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some(rest) = strip_key(line, "uses:") else {
            continue;
        };
        if rest.is_empty() || rest.starts_with("./") {
            continue;
        }

        let (reference, comment) = split_comment(rest);
        let Some((_, revision)) = reference.rsplit_once('@') else {
            violations.push(Violation {
                line: index + 1,
                reason: "no @revision — reference a 40-character commit SHA".to_string(),
                text: line.to_string(),
            });
            continue;
        };

        if !is_commit_sha(revision) {
            violations.push(Violation {
                line: index + 1,
                reason: format!("`{revision}` is a mutable tag, not a 40-character commit SHA"),
                text: line.to_string(),
            });
        } else if comment.is_none() {
            violations.push(Violation {
                line: index + 1,
                reason: "SHA pin has no trailing `# <version>` comment".to_string(),
                text: line.to_string(),
            });
        }
    }

    violations
}

/// Returns every container `image:` reference in `yaml` that is not pinned by
/// an immutable `@sha256:` digest.
fn unpinned_images(yaml: &str) -> Vec<Violation> {
    let mut violations = Vec::new();

    for (index, raw_line) in yaml.lines().enumerate() {
        let line = raw_line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some(rest) = strip_key(line, "image:") else {
            continue;
        };
        let (reference, _) = split_comment(rest);
        if reference.is_empty() {
            continue;
        }

        if !reference.contains("@sha256:") {
            violations.push(Violation {
                line: index + 1,
                reason: "container image is not pinned by @sha256: digest".to_string(),
                text: line.to_string(),
            });
        }
    }

    violations
}

/// Strips a leading YAML list marker and the given key, returning the value.
fn strip_key<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let line = line.strip_prefix("- ").unwrap_or(line).trim_start();
    line.strip_prefix(key).map(str::trim)
}

/// Splits `value  # comment` into the value and the optional comment.
fn split_comment(value: &str) -> (&str, Option<&str>) {
    match value.split_once('#') {
        Some((reference, comment)) => {
            let comment = comment.trim();
            let comment = if comment.is_empty() {
                None
            } else {
                Some(comment)
            };
            (reference.trim(), comment)
        }
        None => (value.trim(), None),
    }
}

fn is_commit_sha(revision: &str) -> bool {
    revision.len() == 40 && revision.chars().all(|c| c.is_ascii_hexdigit())
}

fn workflows_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../.github/workflows")
}

/// Every workflow YAML file in the repository, sorted for stable reporting.
fn workflow_files() -> Vec<PathBuf> {
    let dir = workflows_dir();
    let entries =
        fs::read_dir(&dir).unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()));

    let mut files: Vec<PathBuf> = entries
        .map(|entry| entry.expect("unreadable directory entry").path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("yml") | Some("yaml")
            )
        })
        .collect();
    files.sort();
    files
}

#[test]
fn repository_ships_workflows() {
    let files = workflow_files();
    assert!(
        !files.is_empty(),
        "no workflow YAML found in {} — the CI gates are missing",
        workflows_dir().display()
    );
}

#[test]
fn every_workflow_action_is_sha_pinned() {
    let mut failures = Vec::new();

    for path in workflow_files() {
        let yaml = fs::read_to_string(&path).expect("workflow is readable");
        for violation in unpinned_uses(&yaml) {
            failures.push(format!(
                "{}:{} {} — {}",
                path.display(),
                violation.line,
                violation.text,
                violation.reason
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "unpinned GitHub Actions references:\n{}",
        failures.join("\n")
    );
}

#[test]
fn every_container_image_is_digest_pinned() {
    let mut failures = Vec::new();

    for path in workflow_files() {
        let yaml = fs::read_to_string(&path).expect("workflow is readable");
        for violation in unpinned_images(&yaml) {
            failures.push(format!(
                "{}:{} {} — {}",
                path.display(),
                violation.line,
                violation.text,
                violation.reason
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "unpinned container images:\n{}",
        failures.join("\n")
    );
}

#[test]
fn composite_actions_are_sha_pinned_too() {
    let actions_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.github/actions");
    if !actions_dir.is_dir() {
        // Refinery ships no composite actions; nothing to police.
        return;
    }

    let mut failures = Vec::new();
    let mut stack = vec![actions_dir];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("actions directory is readable") {
            let path = entry.expect("unreadable directory entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if !matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("yml") | Some("yaml")
            ) {
                continue;
            }
            let yaml = fs::read_to_string(&path).expect("action is readable");
            for violation in unpinned_uses(&yaml) {
                failures.push(format!(
                    "{}:{} — {}",
                    path.display(),
                    violation.line,
                    violation.reason
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "unpinned composite action references:\n{}",
        failures.join("\n")
    );
}

#[test]
fn sha_pinned_reference_with_version_comment_passes() {
    let yaml = "      - uses: actions/checkout@93cb6efe18208431cddfb8368fd83d5badbf9bfd  # v5\n";
    assert_eq!(unpinned_uses(yaml), Vec::new());
}

#[test]
fn tag_pinned_reference_is_reported() {
    let yaml = "      - uses: actions/checkout@v5\n";
    let violations = unpinned_uses(yaml);
    assert_eq!(violations.len(), 1, "expected exactly one violation");
    assert_eq!(violations[0].line, 1);
    assert!(
        violations[0].reason.contains("mutable tag"),
        "unexpected reason: {}",
        violations[0].reason
    );
}

#[test]
fn sha_pin_without_version_comment_is_reported() {
    let yaml = "      - uses: actions/checkout@93cb6efe18208431cddfb8368fd83d5badbf9bfd\n";
    let violations = unpinned_uses(yaml);
    assert_eq!(violations.len(), 1);
    assert!(violations[0].reason.contains("trailing"));
}

#[test]
fn reference_without_revision_is_reported() {
    let violations = unpinned_uses("      - uses: actions/checkout\n");
    assert_eq!(violations.len(), 1);
    assert!(violations[0].reason.contains("no @revision"));
}

#[test]
fn local_actions_and_reusable_workflows_are_exempt() {
    let yaml = concat!(
        "      - uses: ./.github/actions/setup\n",
        "    uses: ./.github/workflows/security.yml\n",
    );
    assert_eq!(unpinned_uses(yaml), Vec::new());
}

#[test]
fn commented_out_and_empty_lines_are_ignored() {
    let yaml = concat!(
        "\n",
        "# uses: actions/checkout@v5\n",
        "      # - uses: actions/cache@v4\n",
        "      description: mentions uses: loosely in prose\n",
    );
    assert_eq!(unpinned_uses(yaml), Vec::new());
}

#[test]
fn short_or_non_hex_revision_is_not_accepted_as_a_sha() {
    // 40 characters but not hexadecimal, and a truncated SHA.
    let yaml = concat!(
        "      - uses: some/action@zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz  # v1\n",
        "      - uses: some/action@93cb6efe1820  # v1\n",
    );
    assert_eq!(unpinned_uses(yaml).len(), 2);
}

#[test]
fn digest_pinned_image_passes_and_tag_pinned_image_is_reported() {
    let pinned =
        "      image: semgrep/semgrep@sha256:a9ea2d5621c29d815d90c2a3b2f9571da8972ef4  # v1.86.0\n";
    assert_eq!(unpinned_images(pinned), Vec::new());

    let unpinned = "      image: semgrep/semgrep:latest\n";
    let violations = unpinned_images(unpinned);
    assert_eq!(violations.len(), 1);
    assert!(violations[0].reason.contains("@sha256:"));
}

#[test]
fn empty_input_yields_no_violations() {
    assert_eq!(unpinned_uses(""), Vec::new());
    assert_eq!(unpinned_images(""), Vec::new());
}
