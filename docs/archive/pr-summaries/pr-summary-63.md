## Summary

Removed `--verbose` from the `quality` job's test step in `.github/workflows/ci.yml`. It now runs `cargo test --workspace --all-features -- --test-threads=2`, the same command as the local gate in `quality.sh`. A green PR no longer prints a line for every passing test. Failures still show in full, because `cargo test` prints failure detail without the flag. Closes #63.

## Evidence

This is a CI configuration change with no UI. `refinery/tests/test_step_parity.rs` reads the committed `ci.yml` and `quality.sh`:

- Before the fix, `ci_runs_the_same_cargo_test_as_quality_sh` and `no_gate_runs_cargo_test_verbosely` both **failed**: `ci.yml runs cargo test verbosely … ["cargo test --workspace --all-features --verbose -- --test-threads=2"]`.
- After the fix, all 3 tests pass.

## Quality gate

`./quality.sh` stops at `scripts/test-runlib.sh` (7 passed, 17 failed). The same failure is on `Develop`, and this PR does not touch either file. The `runlib.sh` synced from NEAT-AI-core in #58 needs a `host:` line from `rustc -vV`, and the test's `rustc` shim doesn't print one. That fix is out of scope here, so it has its own follow-up issue: stSoftwareAU/NEAT-AI-Refinery#70.

I ran every remaining stage of the gate by hand, and all pass: bash syntax, shellcheck, `test-auto-version.sh`, markdownlint, actionlint, `cargo deny check`, `cargo fmt --check`, clippy with `-D warnings`, the full `cargo test --workspace --all-features -- --test-threads=2` and `cargo doc` with `-D warnings`.

## Test Plan

- Added `refinery/tests/test_step_parity.rs`:
  - `ci_runs_the_same_cargo_test_as_quality_sh`: the CI test step and `quality.sh` run the same command.
  - `no_gate_runs_cargo_test_verbosely`: neither gate passes `--verbose`/`-v`/`-vv` to cargo.
  - `verbosity_is_read_from_cargo_flags_only`: covers the helpers' edge cases. A flag after `--` is ignored, a YAML `run:` prefix is stripped, and a non-cargo line is not matched.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
