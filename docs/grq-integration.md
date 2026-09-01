# Integrating Refinery into GRQ

Steps 3, 4 and 5 of the [migration principle](../README.md#migration-principle):
Refinery's sampler ran in production behind a rollback switch beside the
TypeScript sampler it was ported from, the [soak evidence](production-soak.md)
carried the cut-over, and — the rollback period having closed with none of the
[conditions that send it back](production-soak.md#what-sends-it-back) observed —
**the TypeScript sampler and the switch have been removed** (#9).

Refinery stays application-agnostic — it knows nothing about GRQ. This page is
the caller's half of the contract: what GRQ passes in, what it reads back, and
what a host still carrying the retired switch sees.

## Configuration

| Variable | Values | Meaning |
| --- | --- | --- |
| `NEAT_AI_REFINERY_BINARY_PATH` | path | the built binary; `neat_ai_refinery` is resolved from `PATH` when unset |

With no executable binary found, the run fails loud naming that variable rather
than sampling some other way — there is no other way left.

`GRQ_SAMPLER_IMPL` is retired. `refinery` — the only sampler there is — and an
unset variable pass untouched; **`typescript`, or any other value, is fatal**,
not ignored. Both `worker/shared/refinery_sampler.sh` (the fleet path) and
GRQ's `src/train/Sampler.ts` (a run started by hand) stop and say the
TypeScript sampler was removed. Honouring `typescript` as "run Refinery" would
leave an operator believing a rollback took effect while the corpus was
produced by the implementation they were trying to roll away from.

```mermaid
flowchart TD
    S[GRQ Sampler.ts] --> R[neat_ai_refinery sample]
    R --> L[(trainData-binary-sampler)]
    L --> E[Creature.evolveDir<br/>unchanged]
    R -.->|non-zero exit| F[run fails loud<br/>nothing to fall back to]
    X{{GRQ_SAMPLER_IMPL=typescript}} -.->|retired switch| Y[fatal before the run starts]
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

- **`--inputs` / `--outputs`** come from the authoritative creature width
  (`NetworkUtil.getEffectiveInputCount()` and `NetworkUtil.OUTPUT_COUNT`) — the
  same numbers the TypeScript sampler read, so a corpus is interpreted with an
  unchanged record shape. A caller without that helper can take the same two
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

The comparison this migration was measured on: both implementations reported one
line of the same shape, and the surviving one still logs it, so runs either side
of the cut-over compare directly without new instrumentation:

```text
🏭 sampler implementation=refinery elapsed_ms=412 records_read=1600000 records_written=80042 output=trainData-binary-sampler/sample-5.bin
🏭 sampler implementation=typescript elapsed_ms=9137 records_read=1600000 records_written=79988 output=trainData-binary-sampler/sample-5.bin
```

`records_read` must match between the two on the same corpus; `records_written`
differs by sampling noise around `rate × records_read`, which is the band the
[parity harness](parity-harness.md) already holds both samplers to.

What that comparison measured, on a corpus of 160 000 production-shaped records,
is in [`production-soak.md`](production-soak.md): 214 ms and 13 MiB peak RSS for
Refinery against 642 ms and 165 MiB for the Deno sampler, both reading the whole
corpus.

## No silent fallback

A failed Refinery run exits non-zero and publishes nothing; the previously
published corpus is left exactly as it was. Nothing re-runs another sampler to
rescue it — while both existed, a hidden fallback would have turned a Rust
failure into a green run whose timings and counts came from the other
implementation, which is precisely the evidence the soak depended on. Since #9
there is no other implementation to rescue it with.

The same rule applies before the run starts: with no executable binary found,
the worker fails loud naming `NEAT_AI_REFINERY_BINARY_PATH`. Every host needs
the binary installed.

## Retrying a full volume

Not every failure is worth retrying, and one is: a sampler run that stopped
because the volume filled up succeeds once space is freed, while every other
failure repeats. Refinery says which it was in the exit code — **28**, POSIX
`ENOSPC`, for a full target volume and **1** for a transform it could not
complete (see the [README](../README.md#exit-codes) for the codes a refused
command line and a panic carry) — so the caller gates its retry on a number
rather than on the wording of an error message Refinery does not promise:

```mermaid
flowchart LR
    R[neat_ai_refinery sample] -->|exit 0| P[(corpus published)]
    R -->|exit 28 + required_bytes<br/>volume full| W{free space >=<br/>required_bytes?}
    W -->|yes| Y[retry the attempt]
    W -->|no| N[fail once, loudly]
    R -->|exit 1<br/>any other failure| F[fail loud, do not retry]
```

### Retrying on evidence, not on hope

The code alone says the volume is full; it cannot say whether another attempt
would fit, and GRQ's gate proved that gap costs a whole stage. On GRQ-19 the
sweep between attempts reclaimed **19 GB**, the gate approved the retry on that
alone, and the pass — which needs about 19 GB — exhausted the volume 97 seconds
later, three times over
([stSoftwareAU/GRQ#4611](https://github.com/stSoftwareAU/GRQ/issues/4611)).

So a stopped sampling run reports what another attempt would have to write:

```text
neat_ai_refinery: trainData-binary-sampler/sample-5.bin: No space left on device (os error 28) — out of space with 4485 of about 7426 records written; another attempt writes the corpus again from the first record: required_bytes=61440
```

| Figure | Who reports it | Why |
| --- | --- | --- |
| `required_bytes` | Refinery | only the run knows the corpus it set out to write |
| free space | the caller | it measures the volume at the moment it decides |

**The figure is the whole corpus, not the remainder.** A partial corpus is never
resumed — the next attempt re-reads every source from the first record, and a
caller sweeping scratch between attempts deletes the partial output before it
starts. Reporting the 40% left of a pass that died 60% through would approve a
retry the volume cannot hold, which is the failure this reporting exists to end.
It is an estimate of the corpus (`rate` × source records), and it is **never
reported as zero**: a pass that plans no records reports no figure at all,
because zero would read as "any volume fits".

The caller then has one rule: retry only when the free space it measures covers
`required_bytes`, and fail once, naming both figures, when it does not or when
no figure was reported. GRQ's half is `grq_sampler_required_kb` in
`worker/shared/sampler_enospc.sh`, which reads the report the attempt it is
judging wrote.

The caller's half is to carry the child's exit code through, unchanged, to the
gate that reads it: `runRefinerySampler` puts the code on the error it throws,
`src/train/Sampler.ts` exits with it, and `worker/shared/sampler_enospc.sh`
retries the attempt it recognises by that 28. A caller that collapses the code
to 1 — or reports the failure as a message alone — burns every attempt on a
disk that only needed room.

## Where it lives in GRQ

| File | Role |
| --- | --- |
| `src/train/RefinerySampler.ts` | the argument builder, the subprocess call, the manifest read |
| `src/train/Sampler.ts` | the CLI entrypoint: resolves the corpus and record shape, calls Refinery, reports the comparable line |
| `worker/shared/refinery_sampler.sh` | grants Deno the scoped `--allow-run` for the binary, or fails loud — and refuses a retired `GRQ_SAMPLER_IMPL` |
| `worker/sampler.sh`, `worker/teams/run.sh` | source the helper and pass the flag through |
| `test/train/RefinerySampler_test.ts`, `test/worker/RefinerySamplerSwitch.ts` | cover the argv, the retired switch, and the fail-loud paths |

NEAT-AI-scorer is untouched: it already scores many creatures in one pass, and
this integration only changes how the corpus those generations reuse is
prepared.
