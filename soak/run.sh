#!/usr/bin/env bash
# Production soak — the evidence gate for making Refinery the GRQ default
# (issue #8).
#
# One run on one host: repeated sampling rounds through the release binary,
# the Deno sampler over the same corpus for comparison, and the invariants the
# cut-over depends on. Everything it needs is checked first and a missing tool
# is a failure, never a skip: a soak that quietly ran half of itself would be
# worse evidence than none.
#
# Usage:
#   ./soak/run.sh                      # production shape, 8×20000 records, rate 0.05
#   ./soak/run.sh --rounds 5           # any production_soak option is passed through
#   ./soak/run.sh --consumer           # also re-check NEAT-AI's evolveDir (needs jsr.io)
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "Refinery production soak"
echo "========================"

if [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

if ! command -v deno &>/dev/null; then
  echo "deno is required — the soak compares Refinery against the Deno sampler." >&2
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

echo "Running the soak..."
cargo run --release --manifest-path "$repo_root/Cargo.toml" \
  --example production_soak -- "$@" < /dev/null

echo "Soak complete — commit the evidence under docs/evidence/."
