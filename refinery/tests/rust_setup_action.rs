//! Drift gate for the shared Rust setup block (Issue #43).
//!
//! "Check out, install the pinned toolchain, restore the Cargo cache" used to
//! be copy-pasted into five workflows. A change to the toolchain pin, the
//! cache-key strategy or the `persist-credentials` policy then had to be
//! applied five times by hand, and an edit that reached only four of the five
//! silently reintroduced the drift the other four were updated to avoid.
//!
//! The toolchain install and the Cargo cache now live once, in
//! `.github/actions/rust-setup`. The checkout cannot move there — the runner
//! reads a local action from the workspace, so the repository must be checked
//! out before the action resolves — so the policy that made it worth
//! centralising is gated directly instead: every checkout in a workflow that
//! calls the action must set `persist-credentials: false`.
//!
//! These tests hold that line: no workflow may inline a Cargo cache again,
//! every Rust workflow must reach its setup through the composite action, none
//! of their checkouts may persist the token, and the cache-key script the
//! action calls is executed here for real so its key ladder is covered by
//! `cargo test` rather than only by a live CI run.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use support::workflow_yaml::invocation_inputs;

/// The workflows that share the Rust setup block, and the cache-key suffix each
/// one used before the extraction. The suffix keeps a workflow's cache distinct
/// from its siblings', so preserving it preserves the cache hits.
/// The three corpus workflows reach it through `_corpus-runner.yml`, which
/// forwards their suffix; `corpus_runner.rs` holds each caller to its own.
const RUST_WORKFLOWS: [(&str, &str); 3] = [
    ("_corpus-runner.yml", "${{ inputs.cache-key-suffix }}"),
    ("cargo-quality.yml", "quality"),
    ("ci.yml", ""),
];

const RUST_SETUP_USES: &str = "./.github/actions/rust-setup";
const CHECKOUT_USES: &str = "actions/checkout";

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

fn workflow_files() -> Vec<PathBuf> {
    let dir = repo_root().join(".github/workflows");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()))
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

fn cache_key_script() -> PathBuf {
    repo_root().join(".github/actions/rust-setup/cache-key.sh")
}

/// Returns the 1-based line numbers where `yaml` names a Cargo cache directory
/// — the dependency sources or `target/` — as a cached path. Comments are
/// ignored — prose about the cache is not a cache step.
fn cargo_cache_paths(yaml: &str) -> Vec<usize> {
    yaml.lines()
        .enumerate()
        .filter(|(_, line)| {
            let line = line.trim();
            !line.starts_with('#')
                && (line.starts_with("~/.cargo/registry")
                    || line.starts_with("~/.cargo/git")
                    || matches!(line, "target" | "target/"))
        })
        .map(|(index, _)| index + 1)
        .collect()
}

/// Runs the cache-key script, returning its stdout. Fails loudly on a non-zero
/// exit so a broken script can never read as a pass.
fn run_cache_key(args: &[&str]) -> String {
    let output = Command::new(cache_key_script())
        .args(args)
        .output()
        .expect("the cache-key script is executable");
    assert!(
        output.status.success(),
        "cache-key.sh {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("cache-key.sh emits UTF-8")
}

#[test]
fn the_composite_action_ships_the_shared_setup_block() {
    let action = fs::read_to_string(repo_root().join(".github/actions/rust-setup/action.yml"))
        .expect("the rust-setup composite action exists");

    for needle in [
        "uses: dtolnay/rust-toolchain@",
        "uses: actions/cache@",
        "~/.cargo/registry",
        "~/.cargo/git",
    ] {
        assert!(
            action.contains(needle),
            "the composite action no longer carries `{needle}` — the callers rely on it"
        );
    }
}

#[test]
fn no_rust_workflow_checkout_persists_the_token() {
    // The checkout is the one part of the setup block the composite action
    // cannot own, so the policy that made it worth centralising is gated here
    // instead. Workflows that push back with the token — `gitleaks.yml` fetches
    // the base ref — are not part of this set and keep their own policy.
    let mut offenders = Vec::new();
    for path in workflow_files() {
        let yaml = fs::read_to_string(&path).expect("workflow is readable");
        if invocation_inputs(&yaml, RUST_SETUP_USES).is_empty() {
            continue;
        }
        for inputs in invocation_inputs(&yaml, CHECKOUT_USES) {
            if inputs.get("persist-credentials").map(String::as_str) != Some("false") {
                offenders.push(path.display().to_string());
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "no step in these workflows pushes back, so the GITHUB_TOKEN must never reach \
         .git/config; checkout persists it in:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn no_workflow_inlines_a_cargo_cache() {
    let mut offenders = Vec::new();
    for path in workflow_files() {
        let yaml = fs::read_to_string(&path).expect("workflow is readable");
        for line in cargo_cache_paths(&yaml) {
            offenders.push(format!("{}:{line}", path.display()));
        }
    }

    assert!(
        offenders.is_empty(),
        "the Cargo cache belongs to `{RUST_SETUP_USES}` alone — inlined again at:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn every_rust_workflow_uses_the_composite_action() {
    for (name, _) in RUST_WORKFLOWS {
        let yaml = workflow(name);
        assert!(
            !invocation_inputs(&yaml, RUST_SETUP_USES).is_empty(),
            "{name} does not call `{RUST_SETUP_USES}` — the setup block has been copy-pasted back"
        );
    }
}

#[test]
fn each_workflow_keeps_its_original_cache_key_suffix() {
    for (name, expected) in RUST_WORKFLOWS {
        let yaml = workflow(name);
        let invocations = invocation_inputs(&yaml, RUST_SETUP_USES);
        let found: Vec<&str> = invocations
            .iter()
            .map(|inputs| {
                inputs
                    .get("cache-key-suffix")
                    .map(String::as_str)
                    .unwrap_or("")
            })
            .collect();

        assert!(
            found.contains(&expected),
            "{name} should pass cache-key-suffix `{expected}` to keep its cache; found {found:?}"
        );
    }
}

#[test]
fn a_suffixed_key_falls_back_to_the_shared_cache() {
    let output = run_cache_key(&["Linux", "bench", "deadbeef", "f00d"]);
    assert_eq!(
        output,
        concat!(
            "key=Linux-cargo-bench-deadbeef\n",
            "restore-keys<<RUST_SETUP_RESTORE_KEYS\n",
            "Linux-cargo-bench-\n",
            "Linux-cargo-\n",
            "RUST_SETUP_RESTORE_KEYS\n",
            "target-key=Linux-rust-target-f00d-bench-deadbeef\n",
            "target-restore-keys<<RUST_SETUP_TARGET_RESTORE_KEYS\n",
            "Linux-rust-target-f00d-bench-\n",
            "Linux-rust-target-f00d-\n",
            "RUST_SETUP_TARGET_RESTORE_KEYS\n",
        ),
        "both ladders must try this workflow's own cache before the shared one"
    );
}

#[test]
fn an_empty_suffix_yields_the_shared_key() {
    let output = run_cache_key(&["macOS", "", "cafe", "beef"]);
    assert_eq!(
        output,
        concat!(
            "key=macOS-cargo-cafe\n",
            "restore-keys<<RUST_SETUP_RESTORE_KEYS\n",
            "macOS-cargo-\n",
            "RUST_SETUP_RESTORE_KEYS\n",
            "target-key=macOS-rust-target-beef-cafe\n",
            "target-restore-keys<<RUST_SETUP_TARGET_RESTORE_KEYS\n",
            "macOS-rust-target-beef-\n",
            "RUST_SETUP_TARGET_RESTORE_KEYS\n",
        ),
        "an empty suffix writes the shared caches the other workflows fall back to"
    );
}

/// Returns the value of the single-line output `name` in `output`.
fn output_value<'a>(output: &'a str, name: &str) -> &'a str {
    let prefix = format!("{name}=");
    output
        .lines()
        .find_map(|line| line.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| panic!("cache-key.sh emitted no `{name}`:\n{output}"))
}

/// Returns the rungs of the multi-line output `name` in `output`.
fn output_ladder(output: &str, name: &str) -> Vec<String> {
    let opener = format!("{name}<<");
    let mut lines = output.lines();
    let delimiter = lines
        .find_map(|line| line.strip_prefix(opener.as_str()))
        .unwrap_or_else(|| panic!("cache-key.sh emitted no `{name}` ladder:\n{output}"));
    lines
        .take_while(|line| *line != delimiter)
        .map(str::to_string)
        .collect()
}

#[test]
fn a_toolchain_change_never_restores_a_stale_target_cache() {
    // `target/` built by one rustc is dead weight to the next, so a toolchain
    // bump must start a fresh target cache rather than fall back to the old
    // one. The dependency sources in `~/.cargo` do not depend on rustc, so
    // their key must not move with it.
    let before = run_cache_key(&["Linux", "bench", "lock", "rustc1"]);
    let after = run_cache_key(&["Linux", "bench", "lock", "rustc2"]);

    assert_eq!(output_value(&before, "key"), output_value(&after, "key"));
    assert_ne!(
        output_value(&before, "target-key"),
        output_value(&after, "target-key")
    );
    let old_key = output_value(&before, "target-key");
    for rung in output_ladder(&after, "target-restore-keys") {
        assert!(
            !old_key.starts_with(&rung),
            "rung `{rung}` would restore the previous toolchain's target cache `{old_key}`"
        );
    }
}

#[test]
fn a_lock_change_still_restores_the_target_cache() {
    // Cargo rebuilds only what a Cargo.lock change touched, so the previous
    // lock's target cache is still the warm start.
    let before = run_cache_key(&["Linux", "", "lock1", "rustc"]);
    let after = run_cache_key(&["Linux", "", "lock2", "rustc"]);
    let old_key = output_value(&before, "target-key");

    assert_ne!(old_key, output_value(&after, "target-key"));
    assert!(
        output_ladder(&after, "target-restore-keys")
            .iter()
            .any(|rung| old_key.starts_with(rung.as_str())),
        "no rung restores the previous lock's target cache `{old_key}`"
    );
}

#[test]
fn the_target_cache_never_shares_a_prefix_with_the_registry_cache() {
    // A registry rung must never match a target entry, or the other way round.
    let output = run_cache_key(&["Linux", "bench", "lock", "rustc"]);
    let target_key = output_value(&output, "target-key");
    let registry_key = output_value(&output, "key");
    for rung in output_ladder(&output, "restore-keys") {
        assert!(
            !target_key.starts_with(&rung),
            "{rung} matches {target_key}"
        );
    }
    for rung in output_ladder(&output, "target-restore-keys") {
        assert!(
            !registry_key.starts_with(&rung),
            "{rung} matches {registry_key}"
        );
    }
}

/// Returns the paths each `actions/cache` step in `yaml` caches, one list per
/// step, read from its `path: |` block.
fn cache_step_paths(yaml: &str) -> Vec<Vec<String>> {
    let mut steps = Vec::new();
    let mut lines = yaml.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim() != "path: |" {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let mut paths = Vec::new();
        while let Some(next) = lines.peek() {
            let next_indent = next.len() - next.trim_start().len();
            if next.trim().is_empty() || next_indent <= indent {
                break;
            }
            paths.push(next.trim().to_string());
            lines.next();
        }
        steps.push(paths);
    }
    steps
}

#[test]
fn the_composite_action_caches_target_under_its_own_key() {
    let action = fs::read_to_string(repo_root().join(".github/actions/rust-setup/action.yml"))
        .expect("the rust-setup composite action exists");

    let caches = invocation_inputs(&action, "actions/cache");
    let keys: Vec<&str> = caches
        .iter()
        .map(|inputs| inputs.get("key").map(String::as_str).unwrap_or(""))
        .collect();
    assert_eq!(
        keys,
        [
            "${{ steps.cache-keys.outputs.key }}",
            "${{ steps.cache-keys.outputs.target-key }}",
        ],
        "the registry and target caches each need their own key"
    );
    assert_eq!(
        caches[1].get("restore-keys").map(String::as_str),
        Some("${{ steps.cache-keys.outputs.target-restore-keys }}")
    );
    assert_eq!(
        cache_step_paths(&action),
        [
            vec!["~/.cargo/registry".to_string(), "~/.cargo/git".to_string()],
            vec!["target".to_string()],
        ],
        "`target/` must be cached apart from the dependency sources"
    );
}

#[test]
fn cache_step_paths_reads_each_block() {
    let yaml = concat!(
        "        path: |\n",
        "          ~/.cargo/registry\n",
        "          ~/.cargo/git\n",
        "        key: a\n",
        "        path: |\n",
        "          target\n",
    );
    assert_eq!(
        cache_step_paths(yaml),
        [
            vec!["~/.cargo/registry".to_string(), "~/.cargo/git".to_string()],
            vec!["target".to_string()],
        ]
    );
    assert_eq!(cache_step_paths(""), Vec::<Vec<String>>::new());
}

#[test]
fn a_missing_argument_fails_loudly() {
    for args in [
        vec![],
        vec!["Linux"],
        vec!["Linux", "bench"],
        vec!["Linux", "bench", "deadbeef"],
    ] {
        let output = Command::new(cache_key_script())
            .args(&args)
            .output()
            .expect("the cache-key script is executable");
        assert!(
            !output.status.success(),
            "cache-key.sh {args:?} should fail rather than emit a half-formed key"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("required"),
            "cache-key.sh {args:?} should say which argument is missing"
        );
    }
}

#[test]
fn a_suffix_that_could_forge_the_output_is_rejected() {
    // The suffix reaches `$GITHUB_OUTPUT`, so anything that could open a new
    // key or close the heredoc must be refused rather than written out.
    for suffix in ["bench\nkey=evil", "a b", "x/y", "$(id)"] {
        let output = Command::new(cache_key_script())
            .args(["Linux", suffix, "deadbeef", "f00d"])
            .output()
            .expect("the cache-key script is executable");
        assert!(
            !output.status.success(),
            "cache-key.sh accepted the unsafe suffix {suffix:?}"
        );
    }
}

#[test]
fn an_unsafe_or_empty_toolchain_hash_is_rejected() {
    // The toolchain hash reaches `$GITHUB_OUTPUT` too, and an empty one — no
    // `rust-toolchain.toml` matched — would silently drop rustc from the key.
    for hash in ["", "a\nkey=evil", "$(id)"] {
        let output = Command::new(cache_key_script())
            .args(["Linux", "bench", "deadbeef", hash])
            .output()
            .expect("the cache-key script is executable");
        assert!(
            !output.status.success(),
            "cache-key.sh accepted the toolchain hash {hash:?}"
        );
    }
}

#[test]
fn a_dotted_suffix_is_accepted() {
    let output = run_cache_key(&["Linux", "bench_v1.2-a", "abc", "def"]);
    assert!(
        output.starts_with("key=Linux-cargo-bench_v1.2-a-abc\n"),
        "unexpected key: {output}"
    );
}

#[test]
fn cargo_cache_paths_ignores_comments_and_prose() {
    let yaml = concat!(
        "          # ~/.cargo/registry is restored by the composite action\n",
        "          description: mentions ~/.cargo/git in prose\n",
    );
    assert_eq!(cargo_cache_paths(yaml), Vec::<usize>::new());

    let inlined = concat!("        path: |\n", "          ~/.cargo/registry\n");
    assert_eq!(cargo_cache_paths(inlined), vec![2]);

    let target = concat!(
        "        path: |\n",
        "          target/\n",
        "          target\n"
    );
    assert_eq!(cargo_cache_paths(target), vec![2, 3]);
}

#[test]
fn invocation_inputs_reads_the_with_block() {
    let yaml = concat!(
        "      - name: Set up Rust\n",
        "        uses: ./.github/actions/rust-setup\n",
        "        with:\n",
        "          cache-key-suffix: bench\n",
        "          components: rustfmt, clippy\n",
        "\n",
        "      - name: Next step\n",
        "        run: echo done\n",
    );
    let invocations = invocation_inputs(yaml, RUST_SETUP_USES);
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0]["cache-key-suffix"], "bench");
    assert_eq!(invocations[0]["components"], "rustfmt, clippy");
}

#[test]
fn invocation_inputs_handles_a_step_without_a_with_block() {
    let yaml = concat!(
        "      - uses: ./.github/actions/rust-setup\n",
        "      - name: Next step\n",
        "        run: echo done\n",
    );
    let invocations = invocation_inputs(yaml, RUST_SETUP_USES);
    assert_eq!(invocations.len(), 1);
    assert!(invocations[0].is_empty());
}

#[test]
fn invocation_inputs_ignores_other_actions_and_empty_input() {
    let yaml = concat!(
        "      - uses: actions/checkout@93cb6efe  # v5\n",
        "        with:\n",
        "          fetch-depth: 0\n",
    );
    assert_eq!(invocation_inputs(yaml, RUST_SETUP_USES), Vec::new());
    assert_eq!(invocation_inputs("", RUST_SETUP_USES), Vec::new());
}

#[test]
fn invocation_inputs_matches_an_action_whatever_its_pin() {
    let yaml = concat!(
        "      - name: Checkout code\n",
        "        uses: actions/checkout@0123456789abcdef  # v9\n",
        "        with:\n",
        "          persist-credentials: false\n",
    );
    let invocations = invocation_inputs(yaml, CHECKOUT_USES);
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0]["persist-credentials"], "false");
}
