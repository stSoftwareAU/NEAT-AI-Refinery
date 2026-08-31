#!/usr/bin/env bash
# Refinery benchmark — throughput, peak RSS and output size (issue #14).
#
# One run on one host: each transform measured over the same synthetic corpus
# through the release binary, and the Deno sampler GRQ shipped measured beside
# it for comparison. Everything it needs is checked first and a missing tool is
# a failure, never a skip: a benchmark that quietly measured half of itself
# would be worse evidence than none.
#
# Usage:
#   ./bench/run.sh                          # production shape, 8×20000 records, rate 0.05
#   ./bench/run.sh --repeats 5              # any benchmark option is passed through
#   ./bench/run.sh --min-speedup 1.5        # fail unless Refinery beats the Deno sampler
#   ./bench/run.sh --baseline docs/evidence/bench-linux-aarch64.json
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "Refinery benchmark"
echo "=================="

if [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

if ! command -v deno &>/dev/null; then
  echo "deno is required — the benchmark compares Refinery against the Deno sampler." >&2
  echo "Install it (https://docs.deno.com/runtime/getting_started/installation/)," >&2
  echo "or pass --no-reference to capture Refinery's numbers alone." >&2
  # --no-reference is an explicit operator decision, so honour it without deno.
  case " $* " in
    *" --no-reference "*) ;;
    *) exit 1 ;;
  esac
else
  deno --version | head -1
fi

echo "Building the release binary..."
cargo build --release --manifest-path "$repo_root/Cargo.toml"

echo "Running the benchmark..."
cargo run --release --manifest-path "$repo_root/Cargo.toml" \
  --example benchmark -- "$@" < /dev/null

echo "Benchmark complete — commit the evidence under docs/evidence/."
