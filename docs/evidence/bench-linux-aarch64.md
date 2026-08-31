# Refinery benchmark

- **tool** — `neat-ai-refinery 0.1.0`
- **host** — `linux` / `aarch64` (6 cpus, unix family)
- **corpus** — 8 shards × 20000 records (1533.2 MiB), 10048 bytes a record
- **repeats** — 3 (the fastest run of each case is reported; peak RSS is the worst)

| case | transform | wall-clock ms | input GiB/s | records/s | peak RSS KiB | output MiB | output/input |
| --- | --- | --- | --- | --- | --- | --- | --- |
| sample | `sample --rate 0.05` | 304 | 4.93 | 526316 | 13292 | 76.8 | 0.050 |
| quantise | `quantise --scheme bfloat16` | 1581 | 0.95 | 101202 | 2980 | 766.6 | 0.500 |
| pipeline | `pipeline sample → quantise` | 370 | 4.05 | 432432 | 13352 | 38.7 | 0.025 |
| typescript | `Sampler.ts --rate 0.05` | 576 | 2.60 | 277778 | 170876 | 77.6 | 0.051 |

Refinery reads 1.9× the records a second the Deno sampler does, at 0.08× its peak RSS.

Peak RSS is sampled — method `proc-vmhwm`.
