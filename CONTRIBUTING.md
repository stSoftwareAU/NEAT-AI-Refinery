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

## Workflow changes

Pin every third-party action to a 40-character commit SHA with a trailing
`# <version>` comment, and every container image by `sha256:` digest carrying
the `:<version>` tag it was resolved from (`image:1.2.3@sha256:…`) — the digest
decides what runs, the tag is what a dependency updater resolves before
rewriting the digest. Mutable tags and tagless digests are both rejected by
`refinery/tests/workflow_pins.rs`. See the "Continuous integration" section of
`README.md` for the gate layout.

A Rust workflow gets its toolchain and Cargo cache from the composite action
`.github/actions/rust-setup` — change the pin or the cache strategy there, not
in the workflow. Pass `cache-key-suffix` to keep a new workflow's cache
distinct; leave it empty only for the job that writes the shared `<os>-cargo-`
cache. `refinery/tests/rust_setup_action.rs` fails the build if a workflow
inlines its own Cargo cache instead.
