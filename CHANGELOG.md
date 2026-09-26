# Changelog

<!-- markdownlint-configure-file { "MD024": { "siblings_only": true } } -->

Notable changes to `neat-ai-refinery` are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions are
those of `refinery/Cargo.toml`, which follows
[Semantic Versioning](https://semver.org/). Fleet hosts rebuild the binary
whenever that version moves; see `CONTRIBUTING.md` for how entries are added.

## [Unreleased]

### Added

- `CHANGELOG.md`, with `refinery/tests/changelog.rs` holding its format (#64).
- `sbom.yml` diffs a PR's SBOM against the latest Develop baseline and annotates
  new components, warning on those not declared in `refinery/Cargo.toml` (#65).

### Changed

- The CI test step runs a quiet `cargo test`, matching `quality.sh` (#63).

## [0.1.3] - 2026-09-21

### Changed

- Weekly Cargo dependency update (#58).

## [0.1.2] - 2026-09-15

### Changed

- Weekly Cargo dependency update (#57).

## [0.1.1] - 2026-09-14

The first version bump. Everything below landed while the crate still read
`0.1.0`, so fleet hosts first rebuilt it at `0.1.1`.

### Added

- `version-increment.yml` bumps the patch on any PR touching the crate, and the
  canonical NEAT-AI-core `scripts/runlib.sh` is kept in sync by a
  `family-sync` job (#53).
- `neat_ai_refinery` installs through `scripts/runlib.sh` (#54).
- Exit 28 prints `required_bytes=<int>`, so GRQ's ENOSPC gate can retry (#51).
- `docs/incident-response.md`: the procedure for an actively-exploited
  advisory (#49).
- Throughput, peak RSS and output-size benchmarks (#14).
- Documentation of the GRQ TypeScript sampler's removal and its rollback
  switch (#9).
- A social preview image in the README (#29).

### Changed

- Workflows share the `.github/actions/rust-setup` composite action (#43).
- The Semgrep image is pinned by digest and version tag (#42).
- Weekly Cargo dependency update (#47).

### Fixed

- A full volume exits 28 rather than 1, so GRQ's ENOSPC retry fires (#38).

## [0.1.0] - 2026-08-31

### Added

- CI and security posture carried over from NEAT-AI's Rust subprojects (#1).
- The fixed-width corpus format and the immutable-source contract (#2).
- Streaming binary corpus reader and writer (#3).
- A port of GRQ's materialised sampler (#4), with a golden parity harness
  against GRQ's `Sampler.ts` (#5).
- A reproducible seed and transformation manifest beside every derived
  corpus (#6).
- The GRQ consumer manifest contract (#7) and the production soak that cut GRQ
  over to Refinery (#8).
- Quantisation (#11) and seeded fuzz augmentation (#12) as composable
  transforms, and pipelines that compose them in explicit order (#13).

### Security

- A derived corpus that overlaps its source, either way, is refused (#4).
