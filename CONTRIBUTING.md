# Contributing

NEAT-AI-Refinery follows the same broad contribution rules as the other
NEAT-AI Rust subprojects.

## Principles

- Preserve the immutable-source contract.
- Prefer evolutionary migration over rewrites that change behaviour and
  architecture simultaneously.
- Add tests before changing externally observable corpus semantics.
- Keep application-specific logic out of this public library.
- Benchmark performance claims.
- Treat malformed or partial binary records as errors.

## Local checks

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Run the full gate before raising a PR — it mirrors `.github/workflows/ci.yml`:

```bash
./quality.sh
```

There is no override for this gate. The one procedure that skips the weekly
dependency cadence — an actively-exploited advisory — still runs every check;
it is written down in
[`docs/incident-response.md`](docs/incident-response.md).

## Crate version and `scripts/runlib.sh`

Fleet hosts rebuild `neat_ai_refinery` only when `refinery/Cargo.toml`'s
version changes, so a PR that lands source at an unchanged version ships
nothing. `version-increment.yml` bumps the patch for you on any PR touching
`refinery/src/**`, `refinery/Cargo.toml` or `Cargo.lock`; bumping it yourself
is fine, and going *backwards* fails the job. `scripts/auto-version.sh` is the
script behind it, taken verbatim from NEAT-AI-Ockham — including its header,
which still cites Ockham's own issue numbers. No job compares it against
Ockham, so unlike `scripts/runlib.sh` that copy is a point-in-time one;
`scripts/test-auto-version.sh` is what holds its behaviour.

`scripts/runlib.sh` is copied byte-for-byte from NEAT-AI-core `Develop` and is
never edited here, and it needs `cargo`, `rustc` and `jq` on the host.
Behaviour changes go to core; the `family-sync` job pushes the refreshed copy
onto your PR branch when the two differ.

## Changelog

Add a line to [`CHANGELOG.md`](CHANGELOG.md) under `## [Unreleased]` with
any change a fleet operator would notice — a terse one is fine — citing the
issue number.

You do not need to release it yourself. After `version-increment.yml` settles
`refinery/Cargo.toml`'s version for the PR — bumping the patch when the PR has
not — `scripts/changelog-release.sh` turns `[Unreleased]` into
`## [x.y.z] - YYYY-MM-DD` for that version and opens a fresh, empty
`[Unreleased]` above it, in the same bot commit as the bump. This only happens
when that version has no heading yet; an empty `[Unreleased]` is never
released, so a bump with nothing under it gets no heading at all. If the
version already has its heading and `[Unreleased]` still holds entries, the
job fails, naming the version — later entries for that PR go straight into its
section instead. On the weekly dependency-refresh PR (branch
`chore/cargo-upgrade`, opened by `cargo-upgrade.yml`), it first adds
`- Weekly Cargo dependency update (#<PR>).` under `### Changed`. On a fork PR
the bot cannot push, so a maintainer lands the bump and the release by hand.
Doing the release by hand yourself — moving the heading before the job runs —
is also fine. `refinery/tests/changelog.rs` keeps releases newest first,
dated, and never ahead of the crate's version.

## Workflow changes

Pin every third-party action to a 40-character commit SHA with a trailing
`# <version>` comment, and every container image by `sha256:` digest carrying
the `:<version>` tag it was resolved from (`image:1.2.3@sha256:…`) — the digest
decides what runs, the tag is what a dependency updater resolves before
rewriting the digest. Mutable tags and tagless digests are both rejected by
`refinery/tests/workflow_pins.rs`. See the "Continuous integration" section of
`README.md` for the gate layout.

A Rust workflow gets its toolchain and Cargo caches — `~/.cargo` and `target/`
— from the composite action `.github/actions/rust-setup`; change the pin or the
cache strategy there, not in the workflow. Pass `cache-key-suffix` to keep a new
workflow's caches distinct; leave it empty only for the job that writes the
shared `<os>-cargo-` and `<os>-rust-target-` caches.
`refinery/tests/rust_setup_action.rs` fails the build if a workflow inlines its
own Cargo or `target/` cache instead. A workflow that runs a corpus script
(`benchmark.yml`, `parity.yml`, `soak.yml`) calls `_corpus-runner.yml` rather
than repeating its setup; `refinery/tests/corpus_runner.rs` holds that line.
