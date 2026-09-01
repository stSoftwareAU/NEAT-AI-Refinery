# Production soak and the GRQ cut-over

Steps 4 and 5 of the [migration principle](../README.md#migration-principle):
Refinery is the sampler GRQ reaches for, and — the rollback period having closed
with none of the conditions below observed — the TypeScript sampler and the
rollback switch have been removed (#9).

This page is the evidence half of that decision — what was measured, on what,
and what would send it back.

```mermaid
flowchart LR
    P[parity harness<br/>issue #5] --> I[integration behind a switch<br/>issue #7]
    I --> S[soak: measured evidence<br/>issue #8]
    S --> D[Refinery is the GRQ default]
    D -->|rollback period closed, issue #9| R[TypeScript sampler removed]
```

## Running a soak

```bash
./soak/run.sh                     # production shape, 8 × 20 000 records, rate 0.05
./soak/run.sh --rounds 5          # any production_soak option passes through
./soak/run.sh --consumer          # also re-check NEAT-AI's evolveDir (needs jsr.io)
cargo run --release --example production_soak -- --help
```

The run builds a synthetic corpus at the production record shape under the
system temporary directory, soaks the **release** binary — the one a fleet host
would run, resolved from `NEAT_AI_REFINERY_BINARY_PATH` when it is set — and
removes the corpus afterwards. No seed is supplied, so each round seeds from
the operating system exactly as production does.

A report is written to `docs/evidence/soak-<os>-<arch>.json` and `.md`. The
name carries the host, so a macOS and a Linux report sit side by side rather
than overwriting each other.

## What a soak asserts

Every check below is fatal — the run exits non-zero and no report is written,
because evidence that records its own breach as a data field is evidence
somebody skims past.

| Check | How |
| --- | --- |
| Sampling succeeds, repeatedly | N rounds through the real binary; a non-zero exit ends the soak |
| Output geometry is valid | each published corpus is re-opened and verified against its own `manifest.json` — shape, byte length, whole records, record count, SHA-256 |
| No source corpus mutation | the source is SHA-256 digested file by file before the first round and after the last, and the digests must match |
| Atomic publication survives failure | a run is forced to fail on a corpus ending mid-record; the previously published corpus must be byte-identical afterwards and no staging or aside directory may be left behind |
| Throughput and peak RSS | both implementations are run as child processes by the same measuring code |
| `evolveDir` still consumes the corpus | optional `--consumer`: `parity/evolve_dir.ts` evolves a creature over the corpus Refinery published |

Peak memory is **sampled, not instrumented**: on Linux the kernel's own
high-water mark (`VmHWM`) is read every 5 ms, and elsewhere `ps` is polled and
the largest sample kept. The method is recorded in the report beside the
number, and a run too short to sample reports `not sampled` rather than zero.

A full volume is the one failure mode the soak does **not** provoke: simulating
ENOSPC portably needs privileges a soak should not ask for. What is provoked
instead is a fatal error mid-run, which exercises the same abort, the same
staging cleanup and the same untouched-live-corpus guarantee. GRQ's own ENOSPC
path is unchanged by the cut-over — a Refinery failure exits non-zero and GRQ
does not rescue it.

## The evidence

| Host | Report |
| --- | --- |
| `linux` / `aarch64`, 6 CPUs — production shape, 8 × 20 000 records | [`soak-linux-aarch64.md`](evidence/soak-linux-aarch64.md) |
| `linux` / `aarch64` — small shape, with the `evolveDir` consumer check | [`soak-linux-aarch64-consumer.md`](evidence/soak-linux-aarch64-consumer.md) |

At the production record shape, 160 000 records of 10 048 bytes:

| Sampler | Elapsed | Records/s | Peak RSS |
| --- | --- | --- | --- |
| Refinery (round 2) | 214 ms | 747 664 | 13 020 KiB |
| Refinery (round 3) | 220 ms | 727 273 | 12 976 KiB |
| Deno `Sampler.ts` | 642 ms | 249 221 | 168 476 KiB |

Both read all 160 000 records. Refinery kept 7 865 / 8 034 / 7 947 and the Deno
sampler 7 976 — the sampling noise around `rate × records_read` the parity
harness already holds both to. Round 1 is slower than the rounds after it
because the corpus has just been written and is not yet in the page cache.

**macOS**: `.github/workflows/soak.yml` runs the same soak on
`macos-latest` and `ubuntu-latest` for every pull request and publishes each
report as an artefact, so the macOS half of the evidence is produced by CI on
the change itself. No macOS report is committed under `docs/evidence/`: the
container this change was made in is Linux-only, and inventing one would be
worse than naming the gap.

## Rolling back

For the length of the rollback period, rollback was one environment variable on
the affected host — `GRQ_SAMPLER_IMPL=typescript` — with no deploy and no
revert. That period has closed, and the fallback it selected was removed with
it (#9), so **there is no environment variable to set any more**: a host that
still exports `GRQ_SAMPLER_IMPL` fails loud rather than sampling as though the
rollback took effect.

Reverting the GRQ removal commit is what a rollback means now. The argument
contract and the manifest fields GRQ reads back are documented in
[`grq-integration.md`](grq-integration.md).

## What sends it back

These were the conditions that would have sent the fleet back to the TypeScript
sampler; none was observed during the rollback period. Any of them is still a
reason to open an issue rather than to patch around it — the response is now a
revert of the removal, not an environment variable:

- a sampling run that exits non-zero without a cause in its own output;
- a published corpus `evolveDir` cannot consume;
- `records_read` differing between the two implementations on one corpus;
- a source corpus whose bytes changed;
- a live corpus directory missing or half-built after a failed run.
