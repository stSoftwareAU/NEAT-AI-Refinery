#!/usr/bin/env bash
# Hermetic tests for scripts/sbom-diff.sh (Issue #65).
#
# Every case runs the real script against throwaway CycloneDX documents and
# crate manifests, then asserts on the exit code and the annotations it prints.
# Nothing here inspects the script's source.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SBOM_DIFF="${SCRIPT_DIR}/sbom-diff.sh"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "${WORK_DIR}"' EXIT

PASSED=0
FAILED=0

if [[ ! -x "${SBOM_DIFF}" ]]; then
  echo "FAIL: sbom-diff.sh not found or not executable: ${SBOM_DIFF}" >&2
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

assert_not_contains() {
  local desc="$1" needle="$2" file="$3"
  if grep -qF -- "${needle}" "${file}"; then
    echo "  FAIL: ${desc}"
    echo "    '${needle}' unexpectedly found in:"
    sed 's/^/      /' "${file}"
    FAILED=$((FAILED + 1))
  else
    echo "  PASS: ${desc}"
    PASSED=$((PASSED + 1))
  fi
}

# Write a CycloneDX document at $1 listing each remaining argument as a purl.
write_sbom() {
  local path="$1"
  shift
  local components="" purl
  for purl in "$@"; do
    components+="${components:+,}{\"type\":\"library\",\"purl\":\"${purl}\"}"
  done
  cat >"${path}" <<EOF
{
  "bomFormat": "CycloneDX",
  "specVersion": "1.3",
  "metadata": {"component": {"purl": "pkg:cargo/neat-ai-refinery@0.1.3?download_url=file://."}},
  "components": [${components}]
}
EOF
}

MANIFEST="${WORK_DIR}/Cargo.toml"
cat >"${MANIFEST}" <<'EOF'
[package]
name = "neat-ai-refinery"
version = "0.1.3"

[dependencies]
clap = { version = "4.6.7", features = ["derive"] }
serde_json = "1.0.151"
shared.workspace = true
renamed = { package = "real-crate", version = "1" }

[dependencies.tabled-dep]
version = "2"

[dev-dependencies]
proptest = "1"

[target.'cfg(unix)'.dependencies]
libc = "0.2"

[features]
not_a_dep = []
EOF

# Run sbom-diff with the given arguments; sets RC, OUT and ERR.
run_diff() {
  OUT="${WORK_DIR}/out"
  ERR="${WORK_DIR}/err"
  bash "${SBOM_DIFF}" "$@" >"${OUT}" 2>"${ERR}" && RC=0 || RC=$?
}

PREV="${WORK_DIR}/prev.cdx.json"
CURR="${WORK_DIR}/curr.cdx.json"

echo "=== an unchanged inventory reports nothing new ==="
write_sbom "${PREV}" "pkg:cargo/clap@4.6.7" "pkg:cargo/anstream@1.0.0"
write_sbom "${CURR}" "pkg:cargo/clap@4.6.7" "pkg:cargo/anstream@1.0.0"
run_diff "${PREV}" "${CURR}" "${MANIFEST}"
assert_eq "identical SBOMs exit 0" "0" "${RC}"
assert_not_contains "no warning is raised" "::warning" "${OUT}"
assert_contains "the summary names zero new components" "0 new component(s)" "${OUT}"

echo ""
echo "=== a version bump of a known component is not new ==="
write_sbom "${CURR}" "pkg:cargo/clap@4.7.0" "pkg:cargo/anstream@1.0.1"
run_diff "${PREV}" "${CURR}" "${MANIFEST}"
assert_eq "a bump exits 0" "0" "${RC}"
assert_not_contains "a bump raises no warning" "::warning" "${OUT}"
assert_not_contains "a bump raises no notice" "::notice" "${OUT}"

echo ""
echo "=== a new transitive component is flagged ==="
write_sbom "${CURR}" "pkg:cargo/clap@4.6.7" "pkg:cargo/anstream@1.0.0" "pkg:cargo/evil-crate@0.0.1"
run_diff "${PREV}" "${CURR}" "${MANIFEST}"
assert_eq "a new transitive component still exits 0" "0" "${RC}"
assert_contains "the new component gets a warning annotation" \
  "::warning title=New transitive SBOM component::pkg:cargo/evil-crate@0.0.1" "${OUT}"
assert_contains "the warning names the manifest it checked" "${MANIFEST}" "${OUT}"
assert_contains "the summary counts it as unexplained" \
  "1 new component(s), 1 not a direct dependency" "${OUT}"

echo ""
echo "=== a new direct dependency is noted, not warned ==="
write_sbom "${PREV}" "pkg:cargo/anstream@1.0.0"
write_sbom "${CURR}" "pkg:cargo/anstream@1.0.0" "pkg:cargo/clap@4.6.7" \
  "pkg:cargo/serde_json@1.0.151" "pkg:cargo/shared@0.1.0" "pkg:cargo/real-crate@1.0.0" \
  "pkg:cargo/tabled-dep@2.0.0" "pkg:cargo/proptest@1.5.0" "pkg:cargo/libc@0.2.1"
run_diff "${PREV}" "${CURR}" "${MANIFEST}"
assert_eq "new direct dependencies exit 0" "0" "${RC}"
assert_not_contains "no direct dependency is warned about" "::warning" "${OUT}"
for crate in clap serde_json shared real-crate tabled-dep proptest libc; do
  assert_contains "${crate} is recognised as a direct dependency" \
    "::notice title=New direct dependency::pkg:cargo/${crate}@" "${OUT}"
done
assert_contains "the summary counts none as unexplained" \
  "7 new component(s), 0 not a direct dependency" "${OUT}"

echo ""
echo "=== a renamed dependency's alias and a feature name are not direct ==="
write_sbom "${CURR}" "pkg:cargo/anstream@1.0.0" "pkg:cargo/renamed@1.0.0" "pkg:cargo/not_a_dep@1.0.0"
run_diff "${PREV}" "${CURR}" "${MANIFEST}"
assert_contains "the alias key is not the crate name" \
  "::warning title=New transitive SBOM component::pkg:cargo/renamed@1.0.0" "${OUT}"
assert_contains "a [features] key is not a dependency" \
  "::warning title=New transitive SBOM component::pkg:cargo/not_a_dep@1.0.0" "${OUT}"

echo ""
echo "=== hyphen and underscore spellings name the same crate ==="
write_sbom "${CURR}" "pkg:cargo/anstream@1.0.0" "pkg:cargo/serde-json@1.0.151"
run_diff "${PREV}" "${CURR}" "${MANIFEST}"
assert_contains "serde-json matches the serde_json manifest key" \
  "::notice title=New direct dependency::pkg:cargo/serde-json@1.0.151" "${OUT}"

echo ""
echo "=== nested components and qualifiers are compared by name ==="
cat >"${CURR}" <<'EOF'
{
  "bomFormat": "CycloneDX",
  "components": [
    {"purl": "pkg:cargo/anstream@1.0.0?download_url=file://.",
     "components": [{"purl": "pkg:cargo/inner-crate@0.1.0"}]}
  ]
}
EOF
run_diff "${PREV}" "${CURR}" "${MANIFEST}"
assert_eq "a nested document exits 0" "0" "${RC}"
assert_contains "a nested component is found" \
  "::warning title=New transitive SBOM component::pkg:cargo/inner-crate@0.1.0" "${OUT}"
assert_not_contains "a qualifier does not make a known component new" "anstream" "${OUT}"

echo ""
echo "=== annotation text cannot break out of its workflow command ==="
write_sbom "${CURR}" "pkg:cargo/anstream@1.0.0" 'pkg:cargo/odd@1.0.0\n::error::injected 100%'
run_diff "${PREV}" "${CURR}" "${MANIFEST}"
assert_contains "newline and percent are encoded" \
  "pkg:cargo/odd@1.0.0%0A::error::injected 100%25" "${OUT}"
assert_eq "the injected text stays on one line" "0" "$(grep -c '^::error::' "${OUT}" || true)"

echo ""
echo "=== malformed and missing inputs fail loud ==="
run_diff "${WORK_DIR}/absent.cdx.json" "${CURR}" "${MANIFEST}"
assert_eq "a missing previous SBOM exits 2" "2" "${RC}"
assert_contains "the missing SBOM is named" "no such SBOM" "${ERR}"

run_diff "${PREV}" "${CURR}" "${WORK_DIR}/absent-Cargo.toml"
assert_eq "a missing manifest exits 2" "2" "${RC}"
assert_contains "the missing manifest is named" "no such manifest" "${ERR}"

printf '{not json' >"${WORK_DIR}/broken.cdx.json"
run_diff "${PREV}" "${WORK_DIR}/broken.cdx.json" "${MANIFEST}"
assert_eq "invalid JSON exits 2" "2" "${RC}"
assert_contains "invalid JSON is reported" "not a CycloneDX JSON document" "${ERR}"

printf '{"bomFormat": "SPDX", "components": []}' >"${WORK_DIR}/spdx.json"
run_diff "${WORK_DIR}/spdx.json" "${CURR}" "${MANIFEST}"
assert_eq "a non-CycloneDX document exits 2" "2" "${RC}"
assert_contains "the wrong format is reported" "not a CycloneDX JSON document" "${ERR}"

run_diff "${PREV}" "${CURR}"
assert_eq "too few arguments exits 2" "2" "${RC}"
assert_contains "too few arguments prints the usage" "usage: sbom-diff.sh" "${ERR}"

echo ""
echo "=== summary: ${PASSED} passed, ${FAILED} failed ==="
[[ "${FAILED}" -eq 0 ]]
