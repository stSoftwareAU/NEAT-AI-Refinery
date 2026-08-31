# Port the current GRQ materialised sampler without behavioural improvements

## Summary

Refinery had the corpus primitives but no transform. This PR ports GRQ's
`src/train/Sampler.ts` as the first one — a port, not a redesign: the
randomised file order, the independent per-record selection, the per-file
shuffle of what was kept, the `sample-<percent>.bin` naming, the staged build
and the rename-before-delete publish are all reproduced as they behave today.
Closes #4.

Added:

- `neat_ai_refinery::sample` — `SampleRate` (validated `0 < rate <= 1`, the
  range `Sampler.ts` enforces, and the `sample-<percent>.bin` naming),
  `SampleRequest`, `sample()` and `SampleOutcome`.
- `StagedCorpus` — builds the derived corpus in `.<output>.staging-<ts>-<pid>`
  beside the destination and publishes it by renaming the previous directory
  aside, renaming the staging directory in, then removing the aside copy. A
  rename that fails rolls the previous corpus back; a `StagedCorpus` dropped
  unpublished removes its own scratch. This is `publishSamplerDir`'s contract:
  a reader resolving the path sees the old corpus or the new one, never an
  empty or half-built slot.
- `neat_ai_refinery::cli` and a real binary —
  `--source --output --inputs --outputs sample --rate [--seed]`, exiting
  non-zero with the failure on stderr.
- `docs/sampling-semantics.md` — the ported behaviour side by side with the
  Deno source, where the Rust port is stricter, and what was deliberately left
  behind (ENOSPC exit 28, the `.in-use.lock` lease, `VersionManager`).
- `refinery/examples/sample_throughput.rs` — a throughput measurement at the
  production record shape.
- Dependencies: `clap` (derive) and `rand`, both already house style in
  `NEAT-AI-Rebase`, `NEAT-AI-Forests`, `NEAT-AI-Lamarck` and
  `NEAT-AI-Discovery`. `cargo deny check` passes.

Two things the port does **not** copy, both noted in the semantics doc: GRQ's
failure path also reclaims the *live* `-sampler` directory (a full-volume
recovery measure); this removes only the scratch it created and leaves the
published corpus intact. And `--seed` is additive — with no seed the run draws
one from the operating system, exactly as `Math.random()` behaves in
production, and reports it so any run can be replayed.

One hazard closed beyond a literal port: publishing renames the whole output
directory aside and deletes it, so a source and output that overlap — either
nesting — are refused before a file is opened. Without that check
`--output <dir holding the source>` would have deleted a source corpus.

```mermaid
flowchart LR
    S[(source .bin shards<br/>read-only)] --> O[shuffled file order]
    O --> B[keep each record<br/>random &lt; rate]
    B --> F[shuffle that file's<br/>kept records]
    F --> W[staging dir<br/>sample-5.bin]
    W --> P[rename aside → rename in → drop aside]
    P --> L[(live derived corpus)]
```

## Evidence

Backend/CLI change — no web interface to screenshot. The evidence is the test
suite, the gate and the binary run below.

`./quality.sh` passes end to end (bash syntax, shellcheck, markdownlint,
actionlint, cargo-deny, fmt, clippy `-D warnings`, 85 tests, rustdoc).

Publishing a real corpus, then the two loud failures:

```text
$ neat_ai_refinery --source $W/trainData-binary --output $W/trainData-binary-sampler \
    --inputs 2 --outputs 1 sample --rate 0.05 --seed 20260831
🏭 /tmp/…/trainData-binary-sampler/sample-5.bin — 160 of 3000 records kept from 3 file(s), seed 20260831
$ ls /tmp/…/trainData-binary-sampler
sample-5.bin          # 1920 bytes = 160 records × 12

$ … sample --rate 1.5
neat_ai_refinery: invalid sample rate 1.5 — the rate must be greater than 0 and at most 1
exit=1
$ … --output $W/trainData-binary sample --rate 0.5
neat_ai_refinery: derived corpus /tmp/…/trainData-binary overlaps the source corpus
/tmp/…/trainData-binary — publishing replaces the whole output directory, and sources are immutable
exit=1
```

### Performance

`cargo run --release --example sample_throughput` — 8 shards × 20 000 records
(1533 MiB) at the production shape (2511 inputs, 1 output), rate 0.05, on a
7-core container:

| Run | Elapsed | Records/s | Read throughput |
| --- | --- | --- | --- |
| cold page cache | 0.425 s | 376 855 | 3611 MiB/s |
| warm page cache | 0.158 s | 1 012 105 | 9699 MiB/s |
| warm page cache | 0.160 s | 1 002 612 | 9608 MiB/s |

No side-by-side Deno figure: `Sampler.ts` imports `NetworkUtil` and
`VersionManager`, so it cannot be run against a synthetic corpus without GRQ's
creature and version state. Beating Deno is not required by this issue.

## Acceptance Criteria

- **met** — Existing sampling semantics documented — evidence:
  `docs/sampling-semantics.md` (behaviour table against `src/train/Sampler.ts`,
  the per-file shuffle, the rounding rule, the deliberate omissions).
- **met** — Rate validation matches existing allowed range — evidence:
  `refinery/src/sample/plan.rs::SampleRate::new` mirrors
  `(rate > 0 && rate <= 1)`;
  `refinery/tests/sample_transform.rs::rejects_a_rate_outside_the_allowed_range`
  and `::accepts_the_range_the_deno_sampler_accepts`.
- **met** — Atomic publish is preserved — evidence:
  `refinery/src/sample/publish.rs::StagedCorpus::publish`;
  `refinery/tests/sample_publish.rs::replaces_a_live_directory_without_emptying_it_in_place`
  and `::stages_outside_the_live_directory_and_publishes_it_whole`.
- **met** — Tests cover source immutability and publish failure cleanup —
  evidence: `refinery/tests/sample_transform.rs::leaves_the_source_corpus_untouched`,
  `::refuses_to_publish_onto_the_source_directory`,
  `::refuses_to_publish_over_a_directory_holding_the_source`,
  `::fails_loud_on_a_partial_record_and_removes_the_staging_directory`;
  `refinery/tests/sample_publish.rs::restores_the_previous_corpus_when_the_publish_fails`
  and `::removes_the_staging_directory_when_it_is_dropped_unpublished`.
- **met** — Performance is at least measured — evidence:
  `refinery/examples/sample_throughput.rs` and the table above.
- **unrequested** — the overlap check rejecting a source nested inside the
  output — reason: publishing deletes the renamed-aside output directory, so
  without it the transform could destroy an immutable source; it guards the
  issue's own atomic-publish requirement rather than adding behaviour.

## Test Plan

New — `refinery/tests/sample_transform.rs` (15 tests):

- rate validation across the allowed and rejected ranges, `NaN` and infinity
  included;
- `sample-<percent>.bin` naming across 1.0, 0.3, 0.05, 0.125 and 0.001;
- a full-rate sample is a permutation of the source; a 0.25 rate over 4000
  records keeps ~1000;
- a seed reproduces a run byte for byte, a different seed reorders it, and an
  unseeded run reports the seed that replays it;
- the source corpus is byte-identical after a run;
- `.bin` shards are read and stray files and nested directories are not;
- republishing replaces a live derived directory whole and leaves no scratch;
- a truncated shard fails loud and removes the staging directory;
- a source with no `.bin` files, and either nesting of source and output, are
  refused.

New — `refinery/tests/sample_publish.rs` (5 tests): staging lives outside the
live directory; publish replaces it whole; a failed publish rolls the previous
corpus back and leaves no aside or staging directory; an abandoned staging
directory is removed on drop; a missing parent is fatal.

New — `refinery/tests/cli_surface.rs` (6 tests): the documented invocation
parses and builds a request carrying the 10 048-byte record shape; `--seed` is
optional; an out-of-range rate and a zero-output shape are rejected by
validation; `--rate` and the subcommand are required.

New — unit tests in `refinery/src/sample/publish.rs` for path handling.

Existing corpus tests are unchanged and still pass.
