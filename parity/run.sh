#!/usr/bin/env bash
# Golden parity harness — Refinery's sampler against GRQ's `Sampler.ts`,
# plus the NEAT-AI `evolveDir` consumer check (issue #5).
#
# Everything the harness needs is checked first and the run fails loud when a
# tool is missing: a skipped parity gate that reports success would be worse
# than no gate at all.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
parity_dir="$repo_root/parity"

echo "Golden parity harness (NEAT-AI-Refinery)"
echo "========================================"

if ! command -v deno &>/dev/null; then
  echo "deno is required — install: https://docs.deno.com/runtime/getting_started/installation/" >&2
  exit 1
fi
deno --version | head -1

echo "Checking the harness scripts..."
(
  cd "$parity_dir"
  deno fmt --check
  deno lint
  deno check grq_sampler.ts evolve_dir.ts
)

echo "Running the parity harness..."
# REFINERY_PARITY_REQUIRED turns a missing `deno` into a failure rather than a
# skip, so this gate cannot pass by not running.
REFINERY_PARITY_REQUIRED=1 cargo test \
  --manifest-path "$repo_root/Cargo.toml" \
  --test parity_harness -- --nocapture --test-threads=2 < /dev/null

echo "Parity harness passed!"
