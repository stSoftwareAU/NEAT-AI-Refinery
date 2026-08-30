# NEAT-AI-Refinery

> **Raw training data goes in unchanged. Reproducible derived corpora come out.** 🏭

NEAT-AI-Refinery is a high-performance Rust tool for producing transformed
training corpora for [NEAT-AI](https://github.com/stSoftwareAU/NEAT-AI).

It owns the boundary:

```text
immutable source training corpus
        │
        ▼
sampling / shuffling / quantisation / fuzzing / validation
        │
        ▼
derived training corpus
```

The source corpus is never modified in place.

## Scope

Refinery is intentionally application-agnostic. It does not know about GRQ,
stocks, observation-version modules, or NEAT evolution policy.

It operates on fixed-width binary records and produces derived corpora from them.

Refinery owns:

- materialised sampling of a source corpus;
- deterministic/reproducible shuffling;
- quantisation and other representation transforms;
- fuzzing/noise injection;
- transformation manifests and provenance;
- strict validation of record boundaries and output artefacts;
- atomic publication of derived corpora.

Refinery does **not** own:

- creature evolution or generation policy — NEAT-AI;
- scorer-side multi-creature evaluation — NEAT-AI-scorer;
- application-specific orchestration or feature generation — downstream users.

## Migration principle

The existing system is working, so migration is deliberately evolutionary:

1. reproduce the current GRQ sampling behaviour;
2. prove compatibility against fixed fixtures;
3. integrate behind a fallback/feature switch;
4. soak in production;
5. remove obsolete code only after the new path is proven;
6. add new transforms such as quantisation and fuzzing afterwards.

No migration issue should combine behavioural changes with the first sampler port.

## Record shape

Refinery should receive the record shape explicitly. A caller such as GRQ may
derive the values from the current fittest creature JSON and pass them in.

For example:

```text
neat_ai_refinery \
  --source /path/to/trainData-binary \
  --output /path/to/trainData-binary-sampler \
  --inputs 2511 \
  --outputs 1 \
  sample --rate 0.05
```

The important contract is simply:

```text
bytes_per_record = (inputs + outputs) * 4
```

for the current Float32 corpus format.

The downstream orchestration layer may use `jq` (or equivalent) to derive
`inputs` and `outputs` from a creature export. Refinery itself should not
parse GRQ version state.

## Design goals

- Rust-first and highly performant.
- Streaming/bounded-memory processing where practical.
- Deterministic operation when a seed is supplied.
- Fail loud on malformed or partial records.
- Preserve source data unchanged.
- Derived artefacts carry enough metadata to reproduce how they were made.
- Behavioural parity before optimisation.
- Benchmarks reported as throughput, peak RSS and output size rather than
  subjective "faster" claims.

## Development

Typical local checks:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Before raising a PR, run the full local gate — it mirrors CI:

```bash
./quality.sh
```

`quality.sh` needs `shellcheck` and `cargo-deny`; `markdownlint-cli2` and
`actionlint` are used when installed and skipped with a notice otherwise
(CI always runs them).

## Continuous integration

PRs into `Develop` (and `milestone/**`) run the `CI` workflow, whose
`ci-required` job is the single aggregated merge gate:

```mermaid
flowchart LR
    V[validation<br/>required files, cargo metadata] --> Q[quality<br/>cargo-deny, fmt, clippy, build, test, doc]
    V --> S[security<br/>rustsec/audit-check]
    SH[shell-checks<br/>bash -n, shellcheck]
    Q --> R[ci-required]
    S --> R
    SH --> R
```

Standalone gates run on PRs against every base branch, so work that bypasses
the full CI graph is still covered:

| Workflow | Gate |
| --- | --- |
| `cargo-quality.yml` | `cargo fmt` and `cargo clippy` |
| `cargo-audit.yml` | RustSec advisories (also weekly on cron) |
| `dependency-review.yml` | new dependencies: vulnerabilities and licences |
| `gitleaks.yml` | secret scanning over the PR commit range |
| `semgrep.yml` | SAST scanning |
| `sbom.yml` | CycloneDX SBOM artefact |
| `actionlint.yml` | workflow YAML lint |
| `markdown-lint.yml` | `markdownlint-cli2` |
| `cargo-upgrade.yml` | weekly dependency-refresh PR |

Every third-party `uses:` reference is pinned to a 40-character commit SHA with
a trailing `# <version>` comment, and container images are pinned by `sha256:`
digest. `refinery/tests/workflow_pins.rs` enforces that on every `cargo test`
run, so an unpinned action fails the build rather than a review.

Refinery has no NEAT-AI-core path dependency, so — unlike the sibling
projects — no workflow checks out a sibling repository.

## Licence

Apache-2.0.
