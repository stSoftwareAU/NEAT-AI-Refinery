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

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The workflows that share the Rust setup block, and the cache-key suffix each
/// one used before the extraction. The suffix keeps a workflow's cache distinct
/// from its siblings', so preserving it preserves the cache hits.
const RUST_WORKFLOWS: [(&str, &str); 5] = [
    ("benchmark.yml", "bench"),
    ("cargo-quality.yml", "quality"),
    ("ci.yml", ""),
    ("parity.yml", "parity"),
    ("soak.yml", "soak"),
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

/// The indentation width of `line`, in spaces.
fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Returns the 1-based line numbers where `yaml` names a Cargo cache directory
/// as a cached path. Comments are ignored — prose about the cache is not a
/// cache step.
fn cargo_cache_paths(yaml: &str) -> Vec<usize> {
    yaml.lines()
        .enumerate()
        .filter(|(_, line)| {
            let line = line.trim();
            !line.starts_with('#')
                && (line.starts_with("~/.cargo/registry") || line.starts_with("~/.cargo/git"))
        })
        .map(|(index, _)| index + 1)
        .collect()
}

/// Returns the `with:` inputs of every `uses: <action>` step in `yaml`, one map
/// per invocation. A step with no `with:` block yields an empty map. The
/// `@<revision>` pin and any trailing comment are ignored, so `actions/checkout`
/// matches whichever SHA it is currently pinned to.
fn invocation_inputs(yaml: &str, action: &str) -> Vec<BTreeMap<String, String>> {
    let mut invocations = Vec::new();
    let mut lines = yaml.lines().peekable();

    while let Some(raw_line) = lines.next() {
        let line = raw_line.trim();
        if line.starts_with('#') {
            continue;
        }
        let stripped = line.strip_prefix("- ").unwrap_or(line);
        let Some(value) = stripped.strip_prefix("uses:") else {
            continue;
        };
        let reference = value.split('#').next().unwrap_or(value).trim();
        let name = reference.split('@').next().unwrap_or(reference);
        if name != action {
            continue;
        }

        // The `with:` block, when present, is a sibling of `uses:` — same
        // indentation, deeper-indented keys beneath it.
        let step_indent = indent_of(raw_line.strip_prefix("- ").unwrap_or(raw_line));
        let mut inputs = BTreeMap::new();
        let mut in_with = false;
        while let Some(next) = lines.peek() {
            let trimmed = next.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                lines.next();
                continue;
            }
            let next_indent = indent_of(next);
            if next_indent < step_indent || (next_indent == step_indent && in_with) {
                break;
            }
            if next_indent == step_indent {
                if trimmed == "with:" {
                    in_with = true;
                    lines.next();
                    continue;
                }
                break;
            }
            if in_with {
                if let Some((key, value)) = trimmed.split_once(':') {
                    inputs.insert(
                        key.trim().to_string(),
                        value.trim().trim_matches('"').to_string(),
                    );
                }
            }
            lines.next();
        }
        invocations.push(inputs);
    }

    invocations
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
    let output = run_cache_key(&["Linux", "bench", "deadbeef"]);
    assert_eq!(
        output,
        concat!(
            "key=Linux-cargo-bench-deadbeef\n",
            "restore-keys<<RUST_SETUP_RESTORE_KEYS\n",
            "Linux-cargo-bench-\n",
            "Linux-cargo-\n",
            "RUST_SETUP_RESTORE_KEYS\n",
        ),
        "the ladder must try this workflow's own cache before the shared one"
    );
}

#[test]
fn an_empty_suffix_yields_the_shared_key() {
    let output = run_cache_key(&["macOS", "", "cafe"]);
    assert_eq!(
        output,
        concat!(
            "key=macOS-cargo-cafe\n",
            "restore-keys<<RUST_SETUP_RESTORE_KEYS\n",
            "macOS-cargo-\n",
            "RUST_SETUP_RESTORE_KEYS\n",
        ),
        "an empty suffix writes the shared cache the other workflows fall back to"
    );
}

#[test]
fn a_missing_argument_fails_loudly() {
    for args in [vec![], vec!["Linux"], vec!["Linux", "bench"]] {
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
            .args(["Linux", suffix, "deadbeef"])
            .output()
            .expect("the cache-key script is executable");
        assert!(
            !output.status.success(),
            "cache-key.sh accepted the unsafe suffix {suffix:?}"
        );
    }
}

#[test]
fn a_dotted_suffix_is_accepted() {
    let output = run_cache_key(&["Linux", "bench_v1.2-a", "abc"]);
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
