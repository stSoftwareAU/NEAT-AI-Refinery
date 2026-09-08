//! Gates the documented emergency dependency-bump path (issue #49).
//!
//! The normal dependency refresh is deliberately slow: `cargo-upgrade.yml`
//! raises one PR a week and the full gate judges it. An advisory that is being
//! actively exploited cannot wait for the next cycle, and an on-call maintainer
//! improvising the bypass under time pressure is how gates get skipped that
//! should not be. The runbook writes that path down; these tests keep it
//! honest — every step it must cover, every gate it must refuse to drop, and
//! every file and workflow it points at.

use std::fs;
use std::path::{Path, PathBuf};

/// The runbook under test.
const RUNBOOK: &str = "docs/incident-response.md";

/// The steps an on-call maintainer must find written down, in this order.
const REQUIRED_HEADINGS: &[&str] = &[
    "## When the fast lane applies",
    "## Who authorises it",
    "## Gates that still must pass",
    "## Raising the expedited PR",
    "## After the fix lands",
];

/// The heading whose section lists the checks the fast lane may never drop.
const GATES_HEADING: &str = "## Gates that still must pass";

/// Checks that stay mandatory however urgent the bump is.
const MANDATORY_GATES: &[&str] = &["cargo audit", "cargo deny check"];

/// The repository root, resolved from the crate this test is compiled in.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Reads a repository file as text.
fn read(relative: &str) -> String {
    fs::read_to_string(repo_root().join(relative)).unwrap_or_else(|error| {
        panic!("read {relative}: {error}");
    })
}

/// The body of the `## `-level section introduced by `heading`, up to the next
/// `## ` heading or the end of the document. Empty when the heading is absent.
fn section<'a>(markdown: &'a str, heading: &str) -> String {
    let mut lines: Vec<&'a str> = Vec::new();
    let mut inside = false;

    for line in markdown.lines() {
        if line.trim_end() == heading {
            inside = true;
            continue;
        }
        if inside && line.starts_with("## ") {
            break;
        }
        if inside {
            lines.push(line);
        }
    }

    lines.join("\n")
}

/// Every Markdown link and image target in `markdown`, ignoring fenced code
/// blocks — a target inside a fence is an example, not a link a reader follows.
fn link_targets(markdown: &str) -> Vec<(usize, String)> {
    let mut targets = Vec::new();
    let mut fenced = false;

    for (index, line) in markdown.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }

        let mut rest = line;
        while let Some(open) = rest.find("](") {
            let after = &rest[open + 2..];
            let Some(close) = after.find(')') else {
                break;
            };
            targets.push((index + 1, after[..close].to_string()));
            rest = &after[close + 1..];
        }
    }

    targets
}

/// Every `<name>.yml` token mentioned anywhere in `markdown`.
fn workflow_names(markdown: &str) -> Vec<(usize, String)> {
    let mut names = Vec::new();

    for (index, line) in markdown.lines().enumerate() {
        for token in line.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '.')) {
            if token.ends_with(".yml") && token.len() > 4 {
                names.push((index + 1, token.to_string()));
            }
        }
    }

    names
}

#[test]
fn the_security_policy_links_to_the_runbook() {
    let policy = read("SECURITY.md");
    let linked = link_targets(&policy)
        .into_iter()
        .any(|(_, target)| target == RUNBOOK || target == "docs/incident-response.md");

    assert!(
        linked,
        "SECURITY.md must link to {RUNBOOK} so the emergency path is reachable \
         from the policy a reporter reads first"
    );
    assert!(
        repo_root().join(RUNBOOK).exists(),
        "{RUNBOOK} must exist in the repository"
    );
}

#[test]
fn the_runbook_covers_every_required_step() {
    let runbook = read(RUNBOOK);
    let mut position = 0usize;

    for heading in REQUIRED_HEADINGS {
        let found = runbook[position..].find(heading).unwrap_or_else(|| {
            panic!(
                "{RUNBOOK} must carry the heading `{heading}`, after the \
                 headings before it in {REQUIRED_HEADINGS:?}"
            )
        });
        position += found + heading.len();
    }
}

#[test]
fn the_runbook_refuses_to_drop_the_security_gates() {
    let runbook = read(RUNBOOK);
    let gates = section(&runbook, GATES_HEADING);

    assert!(
        !gates.trim().is_empty(),
        "{RUNBOOK} must describe the gates under `{GATES_HEADING}`"
    );

    for gate in MANDATORY_GATES {
        assert!(
            gates.contains(gate),
            "the `{GATES_HEADING}` section of {RUNBOOK} must name `{gate}` — \
             an expedited bump may skip the cadence, never the security gates"
        );
    }
}

#[test]
fn the_runbook_links_resolve_to_committed_files() {
    let root = repo_root();
    let docs_dir = root.join("docs");

    let broken: Vec<String> = link_targets(&read(RUNBOOK))
        .into_iter()
        .filter(|(_, target)| {
            !target.starts_with("http://")
                && !target.starts_with("https://")
                && !target.starts_with('#')
                && !target.starts_with("mailto:")
        })
        .filter(|(_, target)| {
            let path = target.split(['#', '?']).next().unwrap_or(target);
            !docs_dir.join(path).exists()
        })
        .map(|(line, target)| format!("{RUNBOOK}:{line} → {target}"))
        .collect();

    assert!(
        broken.is_empty(),
        "{RUNBOOK} links point at files that are not in the repository:\n{}",
        broken.join("\n")
    );
}

#[test]
fn the_runbook_names_only_workflows_that_exist() {
    let workflows = repo_root().join(".github/workflows");

    let missing: Vec<String> = workflow_names(&read(RUNBOOK))
        .into_iter()
        .filter(|(_, name)| !workflows.join(name).exists())
        .map(|(line, name)| format!("{RUNBOOK}:{line} → {name}"))
        .collect();

    assert!(
        missing.is_empty(),
        "{RUNBOOK} names workflows that are not in .github/workflows:\n{}",
        missing.join("\n")
    );
}
