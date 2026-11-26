# Compaction audit — issue #89

## Summary

Recorded the compaction audit in `docs/compaction-audit.md`, with file-and-symbol
evidence distinguishing corpus transforms, buffering and directory cleanup from
whole-creature rewrites. No duplicate compaction was found, so no migration is
required. Closes #89.

## Spec

### Intent and Rationale

- Refinery transforms training corpora. Its creature consumer delegates evolution
  to NEAT-AI; adding local compaction would expand the audited behaviour.

### Essential Design Decisions

- Use the issue's audit-only outcome: preserve runtime code, dependency pins and
  existing tests. No new algorithm, runtime dependency or test is necessary.
- Treat the upstream release prerequisite as a gate on caller migration. With
  no caller to replace, the audit can complete while that rollout remains open.

### Undiscoverable Facts

- On 2026-11-26, core #779 and NEAT-AI#4196 were open. GitHub returned core
  `v0.23.11` and NEAT-AI `v7.1.41` as the latest stable releases; their resolved
  commits are recorded in the audit. No unpublished compaction API was adopted.

## Evidence

- `docs/compaction-audit.md` records the Rust transforms, candidate-named file
  discovery, byte-buffer movement, publication cleanup, both TypeScript scripts,
  examples and measurement entry points inspected at source commit `52e30c2`.
- Verified `parity/deno.json` and `parity/deno.lock`: the existing NEAT-AI range
  resolves to `7.0.16`, exercised by the consumer compatibility tests.
- Issue provenance checked through `gh issue view`:
  - Refinery #89: **Compaction migration 4: audit and converge on shared Rust compaction**.
  - Core #775: **Family-wide DRY: unified Rust creature compaction — staged migration tracker**.
  - Core #779: **Compaction migration 3: publish one native/WASM compaction API with parity gates**.
  - NEAT-AI#4196: **Compaction migration 4A: switch TypeScript compact callers to shared Rust engine**.
- Documentation-only change; no visual surface changed.

## Test Plan

- [x] Audit local Rust and TypeScript paths and classify equivalent-looking operations.
- [x] Verify upstream issue/release state and existing dependency resolutions.
- [x] `timeout 900 ./parity/run.sh < /dev/null` — passed formatting, lint, types
  and all eight tests in `refinery/tests/parity_harness.rs`, including
  `evolve_dir_consumes_a_refinery_published_corpus` and
  `evolve_dir_rejects_a_corpus_refinery_would_never_publish`.
- [x] `timeout 1500 ./quality.sh < /dev/null` — passed shell/script checks,
  Markdown and Actions lint, cargo-deny, Rust formatting, Clippy, workspace tests
  and documentation.
- No tests added, modified or removed; no business rule or duplicated rule changed.

## Merge verification

- CI and code review remain pending.

## Not checked

Upstream creature serialisation parity, compaction idempotence, scorer
non-regression and native/WASM compaction benchmarks were not run: no local
compaction path exists to migrate. No runtime-change risk identified.
