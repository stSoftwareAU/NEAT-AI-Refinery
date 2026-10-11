# Threat model

The reporting route is in [SECURITY.md](../SECURITY.md); the corpus contract
is in the [README](../README.md#corpus-contract). This file is the short brief
for the scanner.

## What this project does and where untrusted input enters

`neat_ai_refinery` is a CLI and library that reads an immutable training
corpus of fixed-width little-endian `f32` records and publishes a derived
corpus (sampled, quantised to `bfloat16`, fuzzed, or an ordered pipeline of
those) with a `manifest.json` beside it. It needs no network. Treat as
untrusted:

- the source corpus files under `--source` (sizes, partial or empty files,
  non-finite values, symlinks and dot-files in the directory);
- the JSON `pipeline --config` file (stages, parameters, seed, version);
- command-line arguments: `--source`, `--output`, `--inputs`, `--outputs`,
  repeatable `--metadata KEY=VALUE`, rates, scales and clamps;
- the existing contents of the `--output` path and its parent directory.

## Components that matter most / least

Most important:

- the source/destination checks (`refinery/src/corpus/source.rs`,
  `derived.rs`, `discovery.rs`): the [immutable source](../README.md#immutable-source)
  rule says no path — relative, `..` or symlinked — may ever write to,
  truncate or remove a source file;
- staging and atomic publication (`refinery/src/transform/staging.rs`), which
  renames the old destination aside and deletes it: anything that makes it
  delete or replace a path other than the requested `--output` matters;
- record framing and shape arithmetic (`corpus/shape.rs`, `reader.rs`,
  `writer.rs`): overflow, partial records, a record spliced across files;
- pipeline config parsing (`refinery/src/pipeline/config.rs`) and the
  manifest writer (`refinery/src/manifest/`).

The crate is `#![forbid(unsafe_code)]`. Lower priority: the bench and soak
harnesses (`refinery/src/bench/`, `refinery/src/soak/`, `refinery/examples/`,
`bench/`, `soak/`) and the Deno parity harness under `parity/`.

## How to exercise it

From `/src`: `cargo test --workspace --all-features -- --test-threads=2` runs
the suite; `refinery/tests/fixtures/` holds small corpora. The binary is
`target/debug/neat_ai_refinery`, for example
`neat_ai_refinery --source DIR --output DIR --inputs 2 --outputs 1 sample --rate 0.5 --seed 1`.

## How you rate severity

- High: writing to, truncating or deleting a source file, or deleting or
  overwriting any path outside the requested `--output`, from crafted
  arguments, config or directory contents (path traversal, symlink races).
- Medium: a panic, unbounded allocation or hang on a malformed corpus or
  config; a derived corpus that is silently wrong (shifted or spliced
  records, an unrecorded transform, a manifest that misstates its source);
  a failed run that leaves a half-written corpus where readers look.
- Low: wrong exit codes or messages, and faults that need the operator to
  point `--output` at a path they already control.

## Anything to leave alone

- Unseeded runs draw their seed from the operating system and record it; that
  is not a reproducibility bug.
- A `.<name>.staging-*` or `<name>.deleting-*` directory left behind when the
  process is killed (rather than failing) is expected; report it only if it
  can be made to land outside the `--output` parent.
