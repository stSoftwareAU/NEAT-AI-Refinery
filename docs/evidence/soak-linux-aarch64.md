# Refinery production soak

- **tool** — `neat-ai-refinery 0.1.0`
- **host** — `linux` / `aarch64` (6 cpus, unix family)
- **corpus** — 8 shards × 20000 records (1533.2 MiB), 10048 bytes a record
- **rate** — 0.05 (no seed, as production runs)

| round | elapsed ms | records/s | peak RSS KiB | read | kept |
| --- | --- | --- | --- | --- | --- |
| 1 | 438 | 365297 | 12612 | 160000 | 7865 |
| 2 | 214 | 747664 | 13020 | 160000 | 8034 |
| 3 | 220 | 727273 | 12976 | 160000 | 7947 |
| typescript | 642 | 249221 | 168476 | 160000 | 7976 |

Peak RSS is sampled — method `proc-vmhwm`.

## Invariants

- **held** — no source corpus mutation: the source digested identically before and after every run
- **held** — output geometry: every published corpus re-verified against its own manifest (3 rounds)
- **held** — atomic publication: a failed run (exit 1) left the published corpus byte-identical and 0 scratch directories behind
- **not run** — evolveDir consumption: the consumer check was not requested on this host
