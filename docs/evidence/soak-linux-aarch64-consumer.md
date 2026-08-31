# Refinery production soak

- **tool** — `neat-ai-refinery 0.1.0`
- **host** — `linux` / `aarch64` (6 cpus, unix family)
- **corpus** — 2 shards × 400 records (0.0 MiB), 12 bytes a record
- **rate** — 0.5 (no seed, as production runs)

| round | elapsed ms | records/s | peak RSS KiB | read | kept |
| --- | --- | --- | --- | --- | --- |
| 1 | 5 | 160000 | 188 | 800 | 377 |
| typescript | 17 | 47059 | 35504 | 800 | 412 |

Peak RSS is sampled — method `proc-vmhwm`.

## Invariants

- **held** — no source corpus mutation: the source digested identically before and after every run
- **held** — output geometry: every published corpus re-verified against its own manifest (1 rounds)
- **held** — atomic publication: a failed run (exit 1) left the published corpus byte-identical and 0 scratch directories behind
- **held** — evolveDir consumed the published corpus: `{"consumed":true,"error":132225.91286755682,"generations":1}`
