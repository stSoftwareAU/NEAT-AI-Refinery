#!/usr/bin/env bash
# Hermetic tests for the canonical scripts/runlib.sh (Issues #54, #53).
#
# `scripts/runlib.sh` is copied byte-for-byte from NEAT-AI-core `Develop` and
# is never edited here, so this file asserts the contract that copy owes
# Refinery: install `neat_ai_refinery` under CARGO_HOME, stamp it with the
# crate semver, remove `target/`, and build nothing when the stamp already
# matches.
#
# No real compile ever happens. A `cargo` shim answers `metadata`, records
# every invocation, and fabricates a release binary for `build` — so "ran no
# cargo build" is read off the log rather than inferred.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUNLIB="${SCRIPT_DIR}/runlib.sh"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "${WORK_DIR}"' EXIT
REAL_PATH="${PATH}"
SHIM_DIR="${WORK_DIR}/shim"
CARGO_LOG="${WORK_DIR}/cargo.log"

CRATE="neat-ai-refinery"
BIN_NAME="neat_ai_refinery"
VERSION="9.9.9"

PASSED=0
FAILED=0

if [[ ! -x "${RUNLIB}" ]]; then
  echo "FAIL: runlib not found or not executable: ${RUNLIB}" >&2
  exit 2
fi

assert_eq() {
  local desc="$1" expected="$2" actual="$3"
  if [[ "${expected}" == "${actual}" ]]; then
    echo "  PASS: ${desc}"
    PASSED=$((PASSED + 1))
  else
    echo "  FAIL: ${desc}"
    echo "    expected: '${expected}'"
    echo "    actual:   '${actual}'"
    FAILED=$((FAILED + 1))
  fi
}

assert_contains() {
  local desc="$1" needle="$2" file="$3"
  if grep -qF -- "${needle}" "${file}"; then
    echo "  PASS: ${desc}"
    PASSED=$((PASSED + 1))
  else
    echo "  FAIL: ${desc}"
    echo "    '${needle}' not found in:"
    sed 's/^/      /' "${file}"
    FAILED=$((FAILED + 1))
  fi
}

# A cargo shim that logs every invocation to $CARGO_LOG, answers `metadata`
# for the crate under $1, and fabricates target/release/<bin> for `build`.
install_shims() {
  local checkout="$1"
  mkdir -p "${SHIM_DIR}"
  cat >"${SHIM_DIR}/cargo" <<EOF
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "\$*" >>"${CARGO_LOG}"
if [[ "\${1:-}" == "metadata" ]]; then
  cat <<'JSON'
{
  "packages": [
    {
      "name": "${CRATE}",
      "version": "${VERSION}",
      "manifest_path": "${checkout}/refinery/Cargo.toml",
      "targets": [
        { "kind": ["lib"], "name": "${BIN_NAME}" },
        { "kind": ["bin"], "name": "${BIN_NAME}" }
      ]
    }
  ],
  "target_directory": "${checkout}/target"
}
JSON
  exit 0
fi
if [[ "\${1:-}" == "build" ]]; then
  mkdir -p "${checkout}/target/release"
  printf 'fabricated binary\n' >"${checkout}/target/release/${BIN_NAME}"
  chmod +x "${checkout}/target/release/${BIN_NAME}"
  exit 0
fi
echo "UNEXPECTED cargo: \$*" >&2
exit 99
EOF
  chmod +x "${SHIM_DIR}/cargo"
  cat >"${SHIM_DIR}/rustc" <<'EOF'
#!/usr/bin/env bash
echo "rustc 1.90.0 (shim)"
EOF
  chmod +x "${SHIM_DIR}/rustc"
}

# A checkout at $1 shaped like this repository: a one-member workspace whose
# member declares an explicit [[bin]] table, exactly as refinery/Cargo.toml
# does. Pass "auto" as $2 to omit that table.
make_checkout() {
  local checkout="$1" bin_table="${2:-explicit}"
  mkdir -p "${checkout}/refinery/src"
  cat >"${checkout}/Cargo.toml" <<'EOF'
[workspace]
members = ["refinery"]
resolver = "2"
EOF
  cat >"${checkout}/refinery/Cargo.toml" <<EOF
[package]
name = "${CRATE}"
version = "${VERSION}"
edition = "2021"

[lib]
name = "${BIN_NAME}"
path = "src/lib.rs"
EOF
  if [[ "${bin_table}" == "explicit" ]]; then
    cat >>"${checkout}/refinery/Cargo.toml" <<EOF

[[bin]]
name = "${BIN_NAME}"
path = "src/main.rs"
EOF
  fi
  printf 'fn main() {}\n' >"${checkout}/refinery/src/main.rs"
  printf '' >"${checkout}/refinery/src/lib.rs"
  mkdir -p "${checkout}/target/release"
  printf 'stale build output\n' >"${checkout}/target/release/marker"
}

# Run runlib from checkout $1 with CARGO_HOME $2, capturing stdout/stderr.
run_runlib() {
  local checkout="$1" cargo_home="$2" out_file="$3" err_file="$4" rc=0
  (
    cd "${checkout}"
    CARGO_HOME="${cargo_home}" PATH="${SHIM_DIR}:${REAL_PATH}" \
      bash "${RUNLIB}"
  ) >"${out_file}" 2>"${err_file}" || rc=$?
  printf '%s' "${rc}"
}

echo "=== fresh install: binary, stamp, target/ removed ==="
CHECKOUT="${WORK_DIR}/fresh"
CARGO_HOME_DIR="${WORK_DIR}/fresh-cargo"
make_checkout "${CHECKOUT}"
install_shims "${CHECKOUT}"
: >"${CARGO_LOG}"
RC="$(run_runlib "${CHECKOUT}" "${CARGO_HOME_DIR}" "${WORK_DIR}/fresh.out" "${WORK_DIR}/fresh.err")"
assert_eq "fresh install exits 0" "0" "${RC}"
assert_eq "stdout is the installed binary path" \
  "${CARGO_HOME_DIR}/bin/${BIN_NAME}" "$(cat "${WORK_DIR}/fresh.out")"
assert_eq "the binary is installed and executable" "yes" \
  "$([[ -x "${CARGO_HOME_DIR}/bin/${BIN_NAME}" ]] && echo yes || echo no)"
assert_eq "the stamp carries the crate version" "${VERSION}" \
  "$(cat "${CARGO_HOME_DIR}/bin/.${CRATE}.version" 2>/dev/null || echo missing)"
assert_eq "target/ is removed after a successful install" "gone" \
  "$([[ -d "${CHECKOUT}/target" ]] && echo present || echo gone)"
assert_contains "the removal names the path and the bytes freed" \
  "[${CRATE}] removed ${CHECKOUT}/target" "${WORK_DIR}/fresh.err"
assert_eq "cargo build ran exactly once" "1" \
  "$(grep -c '^build ' "${CARGO_LOG}" || true)"

echo ""
echo "=== second run at the same version: already installed, no build ==="
make_checkout "${CHECKOUT}"
: >"${CARGO_LOG}"
RC="$(run_runlib "${CHECKOUT}" "${CARGO_HOME_DIR}" "${WORK_DIR}/again.out" "${WORK_DIR}/again.err")"
assert_eq "second run exits 0" "0" "${RC}"
assert_eq "second run prints the installed path" \
  "${CARGO_HOME_DIR}/bin/${BIN_NAME}" "$(cat "${WORK_DIR}/again.out")"
assert_contains "second run names the installed version" \
  "[${CRATE}] already installed v${VERSION}" "${WORK_DIR}/again.err"
assert_eq "second run runs no cargo build" "0" \
  "$(grep -c '^build ' "${CARGO_LOG}" || true)"

echo ""
echo "=== an explicit [[bin]] table costs one cargo metadata call ==="
# The canonical script's no-cargo fast path declines any manifest whose target
# shape it cannot read unambiguously, and an explicit [[bin]] table — which
# refinery/Cargo.toml carries, to name the binary neat_ai_refinery rather than
# neat-ai-refinery — is one of those. The skip is still reached, one
# `cargo metadata` later; nothing is compiled either way.
assert_eq "the explicit [[bin]] shape falls through to cargo metadata" "1" \
  "$(grep -c '^metadata ' "${CARGO_LOG}" || true)"

CHECKOUT_AUTO="${WORK_DIR}/auto"
make_checkout "${CHECKOUT_AUTO}" auto
: >"${CARGO_LOG}"
RC="$(run_runlib "${CHECKOUT_AUTO}" "${CARGO_HOME_DIR}" "${WORK_DIR}/auto.out" "${WORK_DIR}/auto.err")"
assert_eq "the auto-discovered shape exits 0" "0" "${RC}"
assert_contains "the auto-discovered shape reports already installed" \
  "[${CRATE}] already installed v${VERSION}" "${WORK_DIR}/auto.err"
assert_eq "the auto-discovered shape runs no cargo command at all" "0" \
  "$(wc -l <"${CARGO_LOG}" | tr -d ' ')"

echo ""
echo "=== a deleted stamp forces a rebuild ==="
rm -f "${CARGO_HOME_DIR}/bin/.${CRATE}.version"
make_checkout "${CHECKOUT}"
: >"${CARGO_LOG}"
RC="$(run_runlib "${CHECKOUT}" "${CARGO_HOME_DIR}" "${WORK_DIR}/restamp.out" "${WORK_DIR}/restamp.err")"
assert_eq "the rebuild exits 0" "0" "${RC}"
assert_eq "the rebuild ran cargo build" "1" \
  "$(grep -c '^build ' "${CARGO_LOG}" || true)"
assert_eq "the stamp is written again" "${VERSION}" \
  "$(cat "${CARGO_HOME_DIR}/bin/.${CRATE}.version" 2>/dev/null || echo missing)"

echo ""
echo "=== a failed build keeps target/ and the previous install ==="
CHECKOUT_FAIL="${WORK_DIR}/failing"
CARGO_HOME_FAIL="${WORK_DIR}/failing-cargo"
make_checkout "${CHECKOUT_FAIL}"
mkdir -p "${CARGO_HOME_FAIL}/bin"
printf 'previous binary\n' >"${CARGO_HOME_FAIL}/bin/${BIN_NAME}"
chmod +x "${CARGO_HOME_FAIL}/bin/${BIN_NAME}"
printf '0.0.1\n' >"${CARGO_HOME_FAIL}/bin/.${CRATE}.version"
cat >"${SHIM_DIR}/cargo" <<EOF
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "\$*" >>"${CARGO_LOG}"
if [[ "\${1:-}" == "metadata" ]]; then
  cat <<'JSON'
{
  "packages": [
    {
      "name": "${CRATE}",
      "version": "${VERSION}",
      "manifest_path": "${CHECKOUT_FAIL}/refinery/Cargo.toml",
      "targets": [ { "kind": ["bin"], "name": "${BIN_NAME}" } ]
    }
  ],
  "target_directory": "${CHECKOUT_FAIL}/target"
}
JSON
  exit 0
fi
echo "compile error (shim)" >&2
exit 101
EOF
chmod +x "${SHIM_DIR}/cargo"
: >"${CARGO_LOG}"
RC="$(run_runlib "${CHECKOUT_FAIL}" "${CARGO_HOME_FAIL}" "${WORK_DIR}/fail.out" "${WORK_DIR}/fail.err")"
assert_eq "a failed build exits non-zero" "101" "${RC}"
assert_eq "a failed build keeps target/" "present" \
  "$([[ -d "${CHECKOUT_FAIL}/target" ]] && echo present || echo gone)"
assert_eq "a failed build leaves the previous binary in place" "previous binary" \
  "$(cat "${CARGO_HOME_FAIL}/bin/${BIN_NAME}")"
assert_eq "a failed build leaves the previous stamp in place" "0.0.1" \
  "$(cat "${CARGO_HOME_FAIL}/bin/.${CRATE}.version")"

echo ""
echo "=== this repository's own manifest is the shape under test ==="
assert_eq "the workspace names exactly one member" "1" \
  "$(grep -c '^members = \["refinery"\]$' "${REPO_ROOT}/Cargo.toml" || true)"
assert_eq "the crate installs as ${BIN_NAME}" "1" \
  "$(grep -c "^name = \"${CRATE}\"$" "${REPO_ROOT}/refinery/Cargo.toml" || true)"

echo ""
echo "=== summary: ${PASSED} passed, ${FAILED} failed ==="
[[ "${FAILED}" -eq 0 ]]
