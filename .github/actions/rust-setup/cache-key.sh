#!/usr/bin/env bash
# Emits the cache keys and restore-key ladders for the `rust-setup` composite
# action, in `$GITHUB_OUTPUT` syntax — one pair for the dependency sources in
# `~/.cargo`, one for the `target/` build output (Issue #69):
#
#   key=Linux-cargo-bench-<lock-hash>
#   restore-keys<<RUST_SETUP_RESTORE_KEYS
#   Linux-cargo-bench-
#   Linux-cargo-
#   RUST_SETUP_RESTORE_KEYS
#   target-key=Linux-rust-target-<toolchain-hash>-bench-<lock-hash>
#   target-restore-keys<<RUST_SETUP_TARGET_RESTORE_KEYS
#   Linux-rust-target-<toolchain-hash>-bench-
#   Linux-rust-target-<toolchain-hash>-
#   RUST_SETUP_TARGET_RESTORE_KEYS
#
# Each ladder tries the caller's own cache first, then the shared cache every
# Rust workflow writes to or falls back on. An empty suffix *is* that shared
# cache, so it emits the single rung. The target ladder never leaves its
# toolchain: output built by one rustc is dead weight to the next, so a
# `rust-toolchain.toml` change starts a cold target cache. Its `rust-target`
# prefix keeps a registry rung from ever matching a target entry.
#
# Kept out of `action.yml` so the key logic is covered by `cargo test`
# (`refinery/tests/rust_setup_action.rs`) rather than only by a live CI run.
set -euo pipefail

readonly DELIMITER="RUST_SETUP_RESTORE_KEYS"
readonly TARGET_DELIMITER="RUST_SETUP_TARGET_RESTORE_KEYS"

os="${1-}"
suffix="${2-}"
lock_hash="${3-}"
toolchain_hash="${4-}"

if [ "$#" -lt 4 ]; then
  echo "usage: cache-key.sh <runner-os> <cache-key-suffix> <lock-hash> <toolchain-hash> — all four arguments are required" >&2
  exit 2
fi

for named in "os:$os" "lock_hash:$lock_hash" "toolchain_hash:$toolchain_hash"; do
  if [ -z "${named#*:}" ]; then
    echo "cache-key.sh: ${named%%:*} is required and must not be empty" >&2
    exit 2
  fi
done

# The suffix and the hashes are written verbatim into $GITHUB_OUTPUT, so restrict
# them to characters that cannot open a new output key or close the heredoc.
for named in "runner-os:$os" "cache-key-suffix:$suffix" "lock-hash:$lock_hash" "toolchain-hash:$toolchain_hash"; do
  value="${named#*:}"
  if [ -n "$value" ] && ! [[ "$value" =~ ^[A-Za-z0-9._-]+$ ]]; then
    echo "cache-key.sh: ${named%%:*} may only contain letters, digits, '.', '_' and '-' — got '${value}'" >&2
    exit 2
  fi
done

# Prints `<name>=<prefix><lock-hash>` and the `<name>`-restore ladder: the
# caller's own prefix, then the shared one when they differ.
emit() {
  local name="$1" restore_name="$2" delimiter="$3" shared="$4"
  local prefix="$shared"
  if [ -n "$suffix" ]; then
    prefix="${shared}${suffix}-"
  fi

  printf '%s=%s%s\n' "$name" "$prefix" "$lock_hash"
  printf '%s<<%s\n' "$restore_name" "$delimiter"
  printf '%s\n' "$prefix"
  if [ "$prefix" != "$shared" ]; then
    printf '%s\n' "$shared"
  fi
  printf '%s\n' "$delimiter"
}

emit key restore-keys "$DELIMITER" "${os}-cargo-"
emit target-key target-restore-keys "$TARGET_DELIMITER" "${os}-rust-target-${toolchain_hash}-"
