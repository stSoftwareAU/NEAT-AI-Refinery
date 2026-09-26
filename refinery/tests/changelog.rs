//! `CHANGELOG.md` records what each crate version shipped (Issue #64).
//!
//! Fleet hosts rebuild `neat_ai_refinery` whenever `refinery/Cargo.toml`'s
//! version moves, so an operator needs to see what changed between two
//! versions. The changelog follows Keep a Changelog: an `[Unreleased]` section
//! first, then `## [x.y.z] - YYYY-MM-DD` releases, newest first. No release may
//! claim a version the crate has not reached. The current version is not
//! required to have its own heading — `version-increment.yml` bumps the patch
//! on the PR branch, and the entry waits under `[Unreleased]` until then.

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

type Version = (u64, u64, u64);

/// A plain `MAJOR.MINOR.PATCH` — no `v` prefix, pre-release or build suffix.
fn parse_version(text: &str) -> Option<Version> {
    let mut parts = text.split('.').map(|part| {
        let digits = !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
        digits.then(|| part.parse::<u64>().ok()).flatten()
    });
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// A real calendar date written as ISO 8601 `YYYY-MM-DD`.
fn parse_date(text: &str) -> Option<(u32, u32, u32)> {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let field = |range: std::ops::Range<usize>| {
        let part = &text[range];
        part.bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| part.parse::<u32>().ok())
            .flatten()
    };
    let (year, month, day) = (field(0..4)?, field(5..7)?, field(8..10)?);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    (1..=days_in_month)
        .contains(&day)
        .then_some((year, month, day))
}

#[derive(Debug, PartialEq)]
enum Section {
    Unreleased,
    Release {
        version: Version,
        date: (u32, u32, u32),
    },
}

/// Every `## ` heading in `changelog`, parsed; the error names the bad line.
fn sections(changelog: &str) -> Result<Vec<Section>, String> {
    changelog
        .lines()
        .filter_map(|line| line.strip_prefix("## "))
        .map(|heading| {
            let malformed = || format!("malformed release heading: `## {heading}`");
            if heading == "[Unreleased]" {
                return Ok(Section::Unreleased);
            }
            let (bracketed, date) = heading.split_once(" - ").ok_or_else(malformed)?;
            let version = bracketed
                .strip_prefix('[')
                .and_then(|rest| rest.strip_suffix(']'))
                .and_then(parse_version)
                .ok_or_else(malformed)?;
            let date = parse_date(date).ok_or_else(malformed)?;
            Ok(Section::Release { version, date })
        })
        .collect()
}

/// The `version = "…"` of the `[package]` table in a Cargo manifest.
fn package_version(manifest: &str) -> Option<Version> {
    let mut in_package = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package {
            if let Some(value) = line
                .strip_prefix("version")
                .map(str::trim_start)
                .and_then(|rest| rest.strip_prefix('='))
            {
                return parse_version(value.trim().trim_matches('"'));
            }
        }
    }
    None
}

fn changelog_sections() -> Vec<Section> {
    sections(&read("CHANGELOG.md")).unwrap_or_else(|error| panic!("CHANGELOG.md: {error}"))
}

#[test]
fn changelog_opens_with_its_title_and_an_unreleased_section() {
    let changelog = read("CHANGELOG.md");
    assert_eq!(
        changelog.lines().next(),
        Some("# Changelog"),
        "CHANGELOG.md must open with a `# Changelog` title"
    );
    assert_eq!(
        changelog_sections().first(),
        Some(&Section::Unreleased),
        "the first section must be `## [Unreleased]`, where new entries land"
    );
}

#[test]
fn releases_run_newest_first_with_no_repeats() {
    let releases: Vec<_> = changelog_sections()
        .into_iter()
        .skip(1)
        .map(|section| match section {
            Section::Release { version, date } => (version, date),
            Section::Unreleased => panic!("`## [Unreleased]` may appear only once, first"),
        })
        .collect();
    assert!(!releases.is_empty(), "CHANGELOG.md lists no releases");
    for pair in releases.windows(2) {
        let ((newer, newer_date), (older, older_date)) = (pair[0], pair[1]);
        assert!(
            newer > older,
            "{newer:?} must come before {older:?}, strictly newest first"
        );
        assert!(
            newer_date >= older_date,
            "{newer:?} is dated before the older {older:?}"
        );
    }
}

#[test]
fn no_release_is_newer_than_the_crate() {
    let crate_version = package_version(&read("refinery/Cargo.toml"))
        .expect("refinery/Cargo.toml declares a [package] version");
    for section in changelog_sections() {
        if let Section::Release { version, .. } = section {
            assert!(
                version <= crate_version,
                "CHANGELOG.md lists {version:?}, but the crate is only at {crate_version:?}"
            );
        }
    }
}

#[test]
fn contributing_points_at_the_changelog() {
    assert!(
        read("CONTRIBUTING.md").contains("CHANGELOG.md"),
        "CONTRIBUTING.md must tell contributors to add a CHANGELOG.md entry"
    );
}

#[test]
fn versions_are_plain_major_minor_patch() {
    assert_eq!(parse_version("0.1.3"), Some((0, 1, 3)));
    assert_eq!(parse_version("10.20.300"), Some((10, 20, 300)));
    for bad in [
        "",
        "0.1",
        "0.1.3.4",
        "v0.1.3",
        "0.1.3-beta",
        "0..3",
        "0.1.x",
        "+1.2.3",
    ] {
        assert_eq!(parse_version(bad), None, "`{bad}` is not a plain version");
    }
}

#[test]
fn dates_are_real_iso_calendar_days() {
    assert_eq!(parse_date("2026-09-21"), Some((2026, 9, 21)));
    assert_eq!(parse_date("2024-02-29"), Some((2024, 2, 29)));
    for bad in [
        "",
        "2026-9-21",
        "2026/09/21",
        "21-09-2026",
        "2026-13-01",
        "2026-00-10",
        "2026-04-31",
        "2026-02-29",
        "1900-02-29",
        "2026-09-00",
        "2026-09-2x",
    ] {
        assert_eq!(parse_date(bad), None, "`{bad}` is not a real ISO date");
    }
}

#[test]
fn headings_parse_or_name_the_bad_line() {
    let good = "# Changelog\n\n## [Unreleased]\n\n### Added\n\n## [0.1.3] - 2026-09-21\n";
    assert_eq!(
        sections(good),
        Ok(vec![
            Section::Unreleased,
            Section::Release {
                version: (0, 1, 3),
                date: (2026, 9, 21)
            },
        ])
    );
    for bad in [
        "## [0.1.3]",
        "## 0.1.3 - 2026-09-21",
        "## [0.1.3] 2026-09-21",
        "## [v0.1.3] - 2026-09-21",
        "## [0.1.3] - 21/09/2026",
        "## [unreleased]",
        "## Notes",
    ] {
        let error = sections(bad).expect_err(bad);
        assert!(error.contains(bad), "`{error}` should quote `{bad}`");
    }
}

#[test]
fn package_version_reads_only_the_package_table() {
    let manifest =
        "[package]\nname = \"x\"\nversion = \"1.2.3\"\n\n[dependencies]\nversion = \"9.9.9\"\n";
    assert_eq!(package_version(manifest), Some((1, 2, 3)));
    let dependency_only = "[dependencies]\nversion = \"9.9.9\"\n";
    assert_eq!(package_version(dependency_only), None);
    assert_eq!(package_version(""), None);
    assert_eq!(package_version("[package]\nversion = \"1.2\"\n"), None);
}
