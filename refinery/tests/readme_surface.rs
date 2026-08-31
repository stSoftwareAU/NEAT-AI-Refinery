//! Gates on the README's front matter and its in-repository links.
//!
//! The README is the repository's shop window: GitHub renders it on the project
//! page, so the social preview banner belongs at the very top, hot-linked from
//! the NEAT-AI brand directory the sibling projects share. A relative link that
//! points at a file which no longer exists is the other half — documentation
//! that rots silently is worse than none, so both are checked by ordinary tests
//! rather than left for a reader to discover.

use std::fs;
use std::path::{Path, PathBuf};

use clap::CommandFactory;
use neat_ai_refinery::cli::Cli;

/// The raw URL of this project's social preview, served from the NEAT-AI brand
/// directory on `Develop` — the same pattern NEAT-AI-scorer and NEAT-AI-Ockham
/// use, so all the sibling banners move together.
const SOCIAL_PREVIEW_URL: &str = "https://raw.githubusercontent.com/stSoftwareAU/NEAT-AI/Develop/docs/brand/social-previews/neat-ai-refinery.png";

/// The repository root, resolved from the crate this test is compiled in.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// The README as text.
fn readme() -> String {
    fs::read_to_string(repo_root().join("README.md")).expect("read README.md")
}

/// Returns every Markdown link and image target in `markdown`, ignoring the
/// contents of fenced code blocks — a target inside a fence is an example, not
/// a link a reader can follow.
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

#[test]
fn readme_opens_with_the_social_preview_banner() {
    let readme = readme();
    let mut lines = readme.lines().filter(|line| !line.trim().is_empty());

    assert_eq!(
        lines.next(),
        Some("# NEAT-AI-Refinery"),
        "the README must open with the project heading"
    );

    let banner = lines.next().expect("a line after the heading");
    assert!(
        banner.contains(SOCIAL_PREVIEW_URL),
        "the line after the heading must be the social preview banner \
         hot-linked from {SOCIAL_PREVIEW_URL}; found: {banner}"
    );
    assert!(
        banner.contains("!["),
        "the banner must be a Markdown image so GitHub renders it; found: {banner}"
    );
}

#[test]
fn readme_relative_links_resolve_to_committed_files() {
    let root = repo_root();
    let broken: Vec<String> = link_targets(&readme())
        .into_iter()
        .filter(|(_, target)| {
            !target.starts_with("http://")
                && !target.starts_with("https://")
                && !target.starts_with('#')
                && !target.starts_with("mailto:")
        })
        .filter(|(_, target)| {
            let path = target.split(['#', '?']).next().unwrap_or(target);
            !root.join(path).exists()
        })
        .map(|(line, target)| format!("README.md:{line} → {target}"))
        .collect();

    assert!(
        broken.is_empty(),
        "README links point at files that are not in the repository:\n{}",
        broken.join("\n")
    );
}

/// Every long flag the real CLI accepts, globally or on any subcommand.
fn cli_long_flags() -> Vec<String> {
    let command = Cli::command();
    let mut flags: Vec<String> = command
        .get_arguments()
        .filter_map(|arg| arg.get_long())
        .map(|long| format!("--{long}"))
        .collect();

    for subcommand in command.get_subcommands() {
        flags.extend(
            subcommand
                .get_arguments()
                .filter_map(|arg| arg.get_long())
                .map(|long| format!("--{long}")),
        );
    }

    flags.push("--help".to_string());
    flags.push("--version".to_string());
    flags
}

/// Every long flag shown inside a README shell block that invokes the binary.
fn readme_invocation_flags(markdown: &str) -> Vec<(usize, String)> {
    let mut flags = Vec::new();
    let mut in_shell_block = false;
    let mut invoking = false;

    for (index, line) in markdown.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_shell_block = trimmed == "```bash" || trimmed == "```text";
            invoking = false;
            continue;
        }
        if !in_shell_block {
            continue;
        }
        if trimmed.contains("neat_ai_refinery") {
            invoking = true;
        }
        if !invoking {
            continue;
        }
        if !trimmed.ends_with('\\') && !trimmed.contains("neat_ai_refinery") {
            // A continuation line only counts while the invocation is open.
            invoking = trimmed.starts_with('-') || trimmed.starts_with('[');
        }

        for token in trimmed.split_whitespace() {
            let token = token.trim_start_matches('[').trim_end_matches([']', '\\']);
            if token.starts_with("--") && token.len() > 2 {
                flags.push((
                    index + 1,
                    token.split('=').next().unwrap_or(token).to_string(),
                ));
            }
        }
    }

    flags
}

#[test]
fn readme_documents_every_transform_subcommand() {
    let readme = readme();

    for subcommand in Cli::command().get_subcommands() {
        let name = subcommand.get_name();
        assert!(
            readme.contains(&format!("`{name}`")),
            "the README must document the `{name}` subcommand the CLI exposes"
        );
    }
}

#[test]
fn readme_shows_no_flag_the_cli_does_not_accept() {
    let known = cli_long_flags();
    let unknown: Vec<String> = readme_invocation_flags(&readme())
        .into_iter()
        .filter(|(_, flag)| !known.contains(flag))
        .map(|(line, flag)| format!("README.md:{line} → {flag}"))
        .collect();

    assert!(
        unknown.is_empty(),
        "README invocations show flags the CLI does not accept:\n{}\nknown flags: {}",
        unknown.join("\n"),
        known.join(" ")
    );
}
