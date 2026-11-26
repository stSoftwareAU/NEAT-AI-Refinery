# Whole-creature compaction audit

Audit for [#89](https://github.com/stSoftwareAU/NEAT-AI-Refinery/issues/89),
against source commit `52e30c24bfcd253d6b2898512fd9f8d7fa4955fb`.

## Release prerequisite — 2026-11-26

- [NEAT-AI-core#779](https://github.com/stSoftwareAU/NEAT-AI-core/issues/779),
  **Compaction migration 3: publish one native/WASM compaction API with parity
  gates**, was open when checked.
- [NEAT-AI#4196](https://github.com/stSoftwareAU/NEAT-AI/issues/4196),
  **Compaction migration 4A: switch TypeScript compact callers to shared Rust
  engine**, was also open.
- The latest published stable GitHub releases returned were core `v0.23.11`
  ([commit `9ef3668`](https://github.com/stSoftwareAU/NEAT-AI-core/commit/9ef3668def917ffa3266c212d2990f1c77cab27d))
  and NEAT-AI `v7.1.41`
  ([commit `60d6b4e`](https://github.com/stSoftwareAU/NEAT-AI/commit/60d6b4e5d3af20afbaaeb3420d588ad673ab8dab)).
  Their existence does not establish that the required compaction rollout has
  merged and shipped. No compaction caller is switched by this audit.
- `Cargo.toml` and `refinery/Cargo.toml` declare no core dependency.
  `parity/deno.json` requests `jsr:@stsoftware/neat-ai@^7.0.16`, resolved to
  `7.0.16` by `parity/deno.lock`. This is a corpus-consumer harness dependency,
  not a released shared-compaction adapter selected by this issue.

## Audit scope

The tracked Rust library, CLI, examples, test harnesses and TypeScript scripts
are checked for whole-creature rewrites, including equivalent operations under
other names. Corpus transformations and filesystem cleanup are classified
separately from neuron/synapse removal or graph simplification.

## Rust transform and cleanup findings

| Path and symbol | Operation | Classification |
| --- | --- | --- |
| `refinery/src/lib.rs`; `refinery/src/cli.rs::Command`; `refinery/src/main.rs::run` | Expose and dispatch sample, quantise, fuzz and pipeline requests over explicit record shapes. | Corpus API and CLI; no creature graph input. |
| `refinery/src/sample/run.rs::{sample, sample_file}` | Select fixed-width records by rate, then shuffle the kept records. | Corpus sampling, not candidate selection or creature compaction. |
| `refinery/src/quantise/run.rs::quantise` | Transcode record values from float32 to bfloat16, retaining record count and order. | Corpus representation reduction, not neuron weights or graph rewriting. |
| `refinery/src/fuzz/run.rs::{fuzz, perturb_record}` | Apply seeded noise to targeted record values. | Corpus augmentation, not mutation of creature structure. |
| `refinery/src/pipeline/run.rs::{run_pipeline, run_stage}` | Call those transforms in order; remove intermediate stage directories. | Corpus composition and scratch cleanup. |
| `refinery/src/corpus/reader.rs::RecordReader::fill_to_record` | Move unread bytes to the front of a fixed buffer with `copy_within`. | Byte-buffer compaction; no structural rewrite. |
| `refinery/src/corpus/discovery.rs::discover_sources` | Inspect each filesystem `candidate` and retain regular corpus files. | File discovery, not evolved candidate cleanup. |
| `refinery/src/transform/staging.rs::{StagedCorpus::publish, Drop::drop}` | Rename a staged corpus into place and reclaim replaced or abandoned directories. | Filesystem publication and cleanup, not graph pruning. |
| `refinery/src/manifest/model.rs::Manifest` | Serialise corpus provenance, record geometry and caller metadata. | Corpus metadata, not creature JSON, UUIDs or score caches. |

## TypeScript and measurement findings

| Path and symbol | Operation | Classification |
| --- | --- | --- |
| `parity/grq_sampler.ts::{loader, readAndProcess, shuffleUint8Array, shuffleStrings}` | Frozen GRQ reference: select and shuffle binary records. | Corpus parity oracle; no creature import or rewrite. |
| `parity/grq_sampler.ts::publishSamplerDir`; `loader` error cleanup | Rename directories for publication or rollback and remove scratch. | Filesystem cleanup, not candidate cleanup. |
| `parity/evolve_dir.ts::consume` | Construct a small `Creature`, delegate one generation to `Creature.evolveDir`, and require a finite error. | External corpus-consumption check; no local compaction implementation or explicit compaction call. Evolution policy remains inside NEAT-AI. |
| `refinery/tests/parity_harness.rs::{evolve_dir_consumes_a_refinery_published_corpus, evolve_dir_rejects_a_corpus_refinery_would_never_publish}` | Invoke that consumer with published records and a malformed-record control. | Corpus compatibility, not original-versus-compacted creature scoring. |
| `refinery/src/soak/run.rs::{soak, check_consumer, check_atomic_publication}` | Measure corpus sampling, optionally invoke `evolve_dir.ts`, and check failed publication preserves the live corpus. | Measurement and corpus safety; delegates creature work to the same consumer. |
| `refinery/src/bench/run.rs::{bench, measure_case, measure_reference}`; `refinery/examples/{benchmark, production_soak, sample_throughput, quantise_throughput}.rs::main` | Drive corpus transforms and measure throughput, storage or representation error. | Corpus measurement, not a structural compaction benchmark. |
| `parity/run.sh`, `bench/run.sh`, `soak/run.sh` | Run the corresponding harnesses and examples. | Entry-point wiring, with no additional creature rewrite. |

The inventory used `git ls-files` and searches over tracked Rust and TypeScript
for compaction, pruning, simplification, creature/neuron/synapse terminology,
candidate/cleanup paths, graph and score terms, and collection-removal methods.
The relevant implementations above were read, rather than treating the absence
of a method named `compact` as proof. There are two tracked TypeScript scripts;
the Rust module exports and CLI dispatch cover corpus processing and measurement.

## Outcome

**No duplicate whole-creature compaction or equivalent structural rewrite was
found in this checkout. No migration is required.** This is the audit-only case
allowed by [the parent rollout contract](https://github.com/stSoftwareAU/NEAT-AI-core/issues/775).
The open release prerequisites gate a future caller switch; they do not require
adding compaction to Refinery's corpus paths or this consumer check.

This change records the finding without adding a core dependency, switching the
NEAT-AI harness pin, introducing an algorithm or changing runtime behaviour.
Creature serialisation, metadata, UUID/score invalidation, score acceptance,
idempotence and compaction failure semantics have no local implementation to
replace. This audit makes no claim to validate those upstream contracts.
Existing corpus serialisation, caller metadata and publication checks remain
the relevant local safety net. No new test or performance benchmark is warranted
for a documentation-only finding; there is no changed runtime path to compare.

## Validation — 2026-11-26

- `timeout 900 ./parity/run.sh < /dev/null` passed: Deno formatting, lint and
  type checks, followed by all eight existing parity tests. The two
  `evolve_dir_*` tests exercised NEAT-AI consumption and malformed-record
  rejection with the existing `7.0.16` resolution.
- `timeout 1500 ./quality.sh < /dev/null` passed: shell checks and script
  contracts, Markdown and Actions lint, dependency audit, Rust formatting and
  Clippy, workspace tests and documentation. Existing corpus, transform,
  provenance, publication and consumer contracts were retained.
- Not checked: upstream whole-creature serialisation parity, compaction
  idempotence, scorer non-regression or native/WASM compaction performance.
  These belong to the upstream rollout; this audit changes no runtime path.
