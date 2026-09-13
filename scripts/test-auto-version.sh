#!/usr/bin/env bash
# Hermetic tests for scripts/auto-version.sh (Issue #53).
#
# Every case runs the real script against a throwaway manifest and lockfile
# and asserts on the observable outcome — exit code, stdout, stderr, and the
# bytes left on disk. Nothing here inspects the script's source.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AUTO_VERSION="${SCRIPT_DIR}/auto-version.sh"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "${WORK_DIR}"' EXIT

PASSED=0
FAILED=0

if [[ ! -x "${AUTO_VERSION}" ]]; then
  echo "FAIL: auto-version.sh not found or not executable: ${AUTO_VERSION}" >&2
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

# Write a minimal crate manifest at $1 with version $2.
write_manifest() {
  cat >"$1" <<EOF
[package]
name = "neat-ai-refinery"
version = "$2"
edition = "2021"

[dependencies]
clap = { version = "4.6.6" }
EOF
}

# Write a Cargo.lock at $1 carrying neat-ai-refinery at version $2.
write_lockfile() {
  cat >"$1" <<EOF
version = 4

[[package]]
name = "clap"
version = "4.6.6"

[[package]]
name = "neat-ai-refinery"
version = "$2"
dependencies = [
 "clap",
]
EOF
}

manifest_version() {
  awk '/^version = "/ { sub(/^version = "/, ""); sub(/"$/, ""); print; exit }' "$1"
}

# The version recorded for neat-ai-refinery in the lockfile at $1.
lock_version() {
  awk '
    /^\[\[package\]\]/ { in_pkg = 0 }
    $0 == "name = \"neat-ai-refinery\"" { in_pkg = 1; next }
    in_pkg && /^version = / { sub(/^version = "/, ""); sub(/"$/, ""); print; exit }
  ' "$1"
}

echo "=== --print reads the [package] version ==="
MANIFEST="${WORK_DIR}/print-Cargo.toml"
write_manifest "${MANIFEST}" "0.4.11"
OUT="$(bash "${AUTO_VERSION}" --print "${MANIFEST}")"
assert_eq "--print echoes the manifest version" "0.4.11" "${OUT}"

echo ""
echo "=== equal versions bump the patch in manifest and lockfile ==="
MANIFEST="${WORK_DIR}/bump-Cargo.toml"
LOCK="${WORK_DIR}/bump-Cargo.lock"
write_manifest "${MANIFEST}" "0.1.0"
write_lockfile "${LOCK}" "0.1.0"
OUT="$(bash "${AUTO_VERSION}" "${MANIFEST}" "0.1.0" "${LOCK}" 2>"${WORK_DIR}/bump.err")"
assert_eq "bump prints the new version" "0.1.1" "${OUT}"
assert_eq "bump rewrites the manifest" "0.1.1" "$(manifest_version "${MANIFEST}")"
assert_eq "bump rewrites the lockfile entry" "0.1.1" "$(lock_version "${LOCK}")"
assert_contains "bump names the crate and both versions" \
  "bumped neat-ai-refinery 0.1.0 -> 0.1.1" "${WORK_DIR}/bump.err"
assert_eq "the unrelated lockfile entry is untouched" "1" \
  "$(grep -c '^version = "4.6.6"' "${LOCK}")"

echo ""
echo "=== a leading v on the base version is tolerated ==="
MANIFEST="${WORK_DIR}/vprefix-Cargo.toml"
write_manifest "${MANIFEST}" "2.3.4"
OUT="$(bash "${AUTO_VERSION}" "${MANIFEST}" "v2.3.4" 2>/dev/null)"
assert_eq "v-prefixed base still bumps the patch" "2.3.5" "${OUT}"

echo ""
echo "=== a version already ahead of the base is left alone ==="
MANIFEST="${WORK_DIR}/ahead-Cargo.toml"
LOCK="${WORK_DIR}/ahead-Cargo.lock"
write_manifest "${MANIFEST}" "0.2.0"
write_lockfile "${LOCK}" "0.2.0"
OUT="$(bash "${AUTO_VERSION}" "${MANIFEST}" "0.1.9" "${LOCK}" 2>"${WORK_DIR}/ahead.err")"
assert_eq "already-ahead prints the current version" "0.2.0" "${OUT}"
assert_eq "already-ahead leaves the manifest alone" "0.2.0" "$(manifest_version "${MANIFEST}")"
assert_eq "already-ahead leaves the lockfile alone" "0.2.0" "$(lock_version "${LOCK}")"
assert_contains "already-ahead says so on stderr" \
  "already ahead of the base branch" "${WORK_DIR}/ahead.err"

echo ""
echo "=== a downgrade fails loud and changes nothing ==="
MANIFEST="${WORK_DIR}/down-Cargo.toml"
write_manifest "${MANIFEST}" "0.1.4"
OUT="$(bash "${AUTO_VERSION}" "${MANIFEST}" "0.1.9" 2>"${WORK_DIR}/down.err")" && RC=0 || RC=$?
assert_eq "downgrade exits non-zero" "1" "${RC}"
assert_eq "downgrade prints nothing on stdout" "" "${OUT}"
assert_eq "downgrade leaves the manifest alone" "0.1.4" "$(manifest_version "${MANIFEST}")"
assert_contains "downgrade names the direction" "version downgraded: 0.1.9 -> 0.1.4" \
  "${WORK_DIR}/down.err"

echo ""
echo "=== a minor-level downgrade is caught too ==="
MANIFEST="${WORK_DIR}/down-minor-Cargo.toml"
write_manifest "${MANIFEST}" "1.2.9"
bash "${AUTO_VERSION}" "${MANIFEST}" "1.3.0" >/dev/null 2>"${WORK_DIR}/down-minor.err" && RC=0 || RC=$?
assert_eq "minor downgrade exits non-zero" "1" "${RC}"
assert_contains "minor downgrade names the direction" "version downgraded: 1.3.0 -> 1.2.9" \
  "${WORK_DIR}/down-minor.err"

echo ""
echo "=== the patch carries past 9 without touching minor ==="
MANIFEST="${WORK_DIR}/carry-Cargo.toml"
write_manifest "${MANIFEST}" "0.1.9"
OUT="$(bash "${AUTO_VERSION}" "${MANIFEST}" "0.1.9" 2>/dev/null)"
assert_eq "0.1.9 bumps to 0.1.10" "0.1.10" "${OUT}"

echo ""
echo "=== malformed and missing inputs fail loud ==="
MANIFEST="${WORK_DIR}/bad-Cargo.toml"
write_manifest "${MANIFEST}" "0.1"
bash "${AUTO_VERSION}" --print "${MANIFEST}" >/dev/null 2>"${WORK_DIR}/bad.err" && RC=0 || RC=$?
assert_eq "a two-part version exits non-zero" "1" "${RC}"
assert_contains "malformed version is named" "malformed version" "${WORK_DIR}/bad.err"

bash "${AUTO_VERSION}" --print "${WORK_DIR}/absent-Cargo.toml" \
  >/dev/null 2>"${WORK_DIR}/absent.err" && RC=0 || RC=$?
assert_eq "a missing manifest exits non-zero" "1" "${RC}"
assert_contains "missing manifest is named" "no such manifest" "${WORK_DIR}/absent.err"

MANIFEST="${WORK_DIR}/mismatch-Cargo.toml"
LOCK="${WORK_DIR}/mismatch-Cargo.lock"
write_manifest "${MANIFEST}" "0.1.0"
write_lockfile "${LOCK}" "0.1.0"
sed -i.bak 's/name = "neat-ai-refinery"/name = "some-other-crate"/' "${LOCK}"
bash "${AUTO_VERSION}" "${MANIFEST}" "0.1.0" "${LOCK}" \
  >/dev/null 2>"${WORK_DIR}/mismatch.err" && RC=0 || RC=$?
assert_eq "a lockfile without the crate exits non-zero" "1" "${RC}"
assert_contains "the missing lockfile entry is named" "expected exactly one [[package]] entry" \
  "${WORK_DIR}/mismatch.err"

bash "${AUTO_VERSION}" >/dev/null 2>"${WORK_DIR}/usage.err" && RC=0 || RC=$?
assert_eq "no arguments exits non-zero" "1" "${RC}"
assert_contains "no arguments prints the usage" "usage: auto-version.sh" "${WORK_DIR}/usage.err"

echo ""
echo "=== summary: ${PASSED} passed, ${FAILED} failed ==="
[[ "${FAILED}" -eq 0 ]]
