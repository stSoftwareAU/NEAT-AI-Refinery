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
