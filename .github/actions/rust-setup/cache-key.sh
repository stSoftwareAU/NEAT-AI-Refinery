#!/usr/bin/env bash
# Emits the Cargo cache key and its restore-key ladder for the `rust-setup`
# composite action, in `$GITHUB_OUTPUT` syntax:
#
#   key=Linux-cargo-bench-<hash>
#   restore-keys<<RUST_SETUP_RESTORE_KEYS
#   Linux-cargo-bench-
#   Linux-cargo-
#   RUST_SETUP_RESTORE_KEYS
#
# The ladder tries the caller's own cache first, then the shared
# `<os>-cargo-` cache every Rust workflow writes to or falls back on. An empty
# suffix *is* that shared cache, so it emits the single rung.
#
# Kept out of `action.yml` so the key logic is covered by `cargo test`
# (`refinery/tests/rust_setup_action.rs`) rather than only by a live CI run.
set -euo pipefail

readonly DELIMITER="RUST_SETUP_RESTORE_KEYS"

os="${1-}"
suffix="${2-}"
lock_hash="${3-}"

if [ "$#" -lt 3 ]; then
  echo "usage: cache-key.sh <runner-os> <cache-key-suffix> <lock-hash> — all three arguments are required" >&2
  exit 2
fi

for named in "os:$os" "lock_hash:$lock_hash"; do
  if [ -z "${named#*:}" ]; then
    echo "cache-key.sh: ${named%%:*} is required and must not be empty" >&2
    exit 2
  fi
done

# The suffix and the hash are written verbatim into $GITHUB_OUTPUT, so restrict
# them to characters that cannot open a new output key or close the heredoc.
for named in "runner-os:$os" "cache-key-suffix:$suffix" "lock-hash:$lock_hash"; do
  value="${named#*:}"
  if [ -n "$value" ] && ! [[ "$value" =~ ^[A-Za-z0-9._-]+$ ]]; then
    echo "cache-key.sh: ${named%%:*} may only contain letters, digits, '.', '_' and '-' — got '${value}'" >&2
    exit 2
  fi
done

shared="${os}-cargo-"
prefix="$shared"
if [ -n "$suffix" ]; then
  prefix="${shared}${suffix}-"
fi

printf 'key=%s%s\n' "$prefix" "$lock_hash"
printf 'restore-keys<<%s\n' "$DELIMITER"
printf '%s\n' "$prefix"
if [ "$prefix" != "$shared" ]; then
  printf '%s\n' "$shared"
fi
printf '%s\n' "$DELIMITER"
