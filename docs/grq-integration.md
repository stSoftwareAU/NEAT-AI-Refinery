# Integrating Refinery into GRQ

Step 3 of the [migration principle](../README.md#migration-principle):
Refinery's sampler runs in production **behind a rollback switch**, beside the
TypeScript sampler it was ported from, so the two can be compared before
anything is removed.

Refinery stays application-agnostic — it knows nothing about GRQ. This page is
the caller's half of the contract: what GRQ passes in, what it reads back, and
how a fleet run is rolled back.

## The switch

| Variable | Values | Meaning |
| --- | --- | --- |
| `GRQ_SAMPLER_IMPL` | unset / `typescript` (default) | GRQ's `src/train/Sampler.ts` produces the corpus — the current production path, unchanged |
| | `refinery` | `neat_ai_refinery sample` produces the corpus |
| | anything else | fatal: an unrecognised value is a typo, not a selection |
| `NEAT_AI_REFINERY_BINARY_PATH` | path | the built binary; `neat_ai_refinery` is resolved from `PATH` when unset |

Rolling back is unsetting `GRQ_SAMPLER_IMPL` — no deploy, no revert, and the
TypeScript sampler is still the default until the comparison says otherwise.

```mermaid
flowchart TD
    S{{GRQ_SAMPLER_IMPL}} -->|unset / typescript| T[Sampler.ts<br/>TypeScript sampler]
    S -->|refinery| R[neat_ai_refinery sample]
    S -->|anything else| X[fatal: unrecognised switch]
    T --> L[(trainData-binary-sampler)]
    R --> L
    L --> E[Creature.evolveDir<br/>unchanged]
    R -.->|non-zero exit| F[run fails loud<br/>never re-run on the old path]
```

## What GRQ passes in

GRQ remains the orchestration layer. It resolves the source corpus and the
record shape and hands both to Refinery, which never parses GRQ version state:

```bash
neat_ai_refinery \
  --source  trainData-binary \
  --output  trainData-binary-sampler \
  --inputs  2511 \
  --outputs 1 \
  --metadata grq_observation_version=42 \
  sample --rate 0.05
```

- **`--inputs` / `--outputs`** come from the same authoritative creature width
  the TypeScript sampler reads (`NetworkUtil.getEffectiveInputCount()` and
  `NetworkUtil.OUTPUT_COUNT`), so both implementations interpret the corpus with
  an identical record shape. A caller without that helper can take the same two
  numbers from a creature export with `jq '.input, .output'` — they are
  authoritative top-level integers, and a value below 1 is fatal.
- **`--metadata`** carries GRQ's observation version into the manifest verbatim.
  Refinery does not interpret it.
- **`--seed`** is omitted in production, so the run seeds from the operating
  system exactly as `Math.random()` does. The seed used is always reported, so
  any run can be replayed.
- The output directory is the same `-sampler` directory `evolveDir` already
  consumes, published atomically — nothing downstream changes.

## What GRQ reads back

The record counts come from the published
[`manifest.json`](../README.md#transformation-manifest), not from parsing
console output:

| Manifest field | Used for |
| --- | --- |
| `output.file` | the published corpus file name |
| `output.record_count` | records kept — the sample size |
| `source.record_count` | records read — the corpus the sample was drawn from |

Those three key names are a public interface. `refinery/tests/consumer_contract.rs`
asserts them over the raw JSON, so a rename that leaves the Rust struct intact
still fails the build rather than silently blinding the caller.

A published directory whose manifest cannot be read is treated as a failed run:
an unmeasurable corpus would defeat the comparison this integration exists for.

## Comparing the two samplers

Both implementations report one line of the same shape, so a fleet run can be
compared directly without new instrumentation:

```text
🏭 sampler implementation=refinery elapsed_ms=412 records_read=1600000 records_written=80042 output=trainData-binary-sampler/sample-5.bin
🏭 sampler implementation=typescript elapsed_ms=9137 records_read=1600000 records_written=79988 output=trainData-binary-sampler/sample-5.bin
```

`records_read` must match between the two on the same corpus; `records_written`
differs by sampling noise around `rate × records_read`, which is the band the
[parity harness](parity-harness.md) already holds both samplers to.

## No silent fallback

A failed Refinery run exits non-zero and publishes nothing; the previously
published corpus is left exactly as it was. GRQ does **not** re-run the
TypeScript sampler to rescue it. A hidden fallback would turn a Rust failure
into a green run whose timings and counts came from the other implementation,
which is precisely the evidence the soak depends on.

The same rule applies before the run starts: with the switch on and no
executable binary found, the worker fails loud naming
`NEAT_AI_REFINERY_BINARY_PATH` instead of quietly using the old path.

## Where it lives in GRQ

| File | Role |
| --- | --- |
| `src/train/RefinerySampler.ts` | the switch, the argument builder, the subprocess call, the manifest read |
| `src/train/Sampler.ts` | dispatches to the selected implementation and reports the comparable line |
| `worker/shared/refinery_sampler.sh` | grants Deno the scoped `--allow-run` for the binary, or fails loud |
| `worker/sampler.sh`, `worker/teams/run.sh` | source the helper and pass the flag through |
| `test/train/RefinerySampler_test.ts`, `test/worker/RefinerySamplerSwitch.ts` | cover the switch, the argv, and the fail-loud paths |

NEAT-AI-scorer is untouched: it already scores many creatures in one pass, and
this integration only changes how the corpus those generations reuse is
prepared.
