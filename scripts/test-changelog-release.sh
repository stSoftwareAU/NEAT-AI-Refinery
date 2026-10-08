#!/usr/bin/env bash
# Hermetic tests for scripts/changelog-release.sh (Issue #80).
#
# Every case runs the real script against a throwaway CHANGELOG.md and
# asserts on the observable outcome — exit code, stderr, and the exact bytes
# left on disk. Nothing here inspects the script's source.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHANGELOG_RELEASE="${SCRIPT_DIR}/changelog-release.sh"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "${WORK_DIR}"' EXIT

PASSED=0
FAILED=0

if [[ ! -x "${CHANGELOG_RELEASE}" ]]; then
  echo "FAIL: changelog-release.sh not found or not executable: ${CHANGELOG_RELEASE}" >&2
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

# Compare a whole file against expected content given on stdin.
assert_file_eq() {
  local desc="$1" file="$2" expected_file="$3"
  if cmp -s "${file}" "${expected_file}"; then
    echo "  PASS: ${desc}"
    PASSED=$((PASSED + 1))
  else
    echo "  FAIL: ${desc}"
    echo "    diff (expected vs actual):"
    diff -u "${expected_file}" "${file}" | sed 's/^/      /' || true
    FAILED=$((FAILED + 1))
  fi
}

# shellcheck disable=SC2016 # literal markdown backticks, no expansion wanted
PREAMBLE='# Changelog

<!-- markdownlint-configure-file { "MD024": { "siblings_only": true } } -->

Notable changes to `neat-ai-refinery` are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions are
those of `refinery/Cargo.toml`, which follows
[Semantic Versioning](https://semver.org/). Fleet hosts rebuild the binary
whenever that version moves; see `CONTRIBUTING.md` for how entries are added.'

OLDER='## [0.1.3] - 2026-09-21

### Changed

- Weekly Cargo dependency update (#58).'

# Write a fixture CHANGELOG.md at $1, with $2 as the body of [Unreleased]
# (no leading/trailing blank handling done here — caller controls it) and a
# previously released [0.1.4] section with its own ### Changed entries.
write_fixture() {
  local file="$1" unreleased_body="$2"
  {
    printf '%s\n' "${PREAMBLE}"
    echo
    echo "## [Unreleased]"
    if [[ -n "${unreleased_body}" ]]; then
      echo
      printf '%s\n' "${unreleased_body}"
    fi
    echo
    echo "## [0.1.4] - 2026-10-05"
    echo
    echo "### Changed"
    echo
    echo "- Something already released (#70)."
    echo
    printf '%s\n' "${OLDER}"
  } >"${file}"
}

echo "=== 1: Unreleased with Added and Changed entries is released ==="
FILE="${WORK_DIR}/case1.md"
write_fixture "${FILE}" "### Added

- A new thing (#90).

### Changed

- A changed thing (#91)."
bash "${CHANGELOG_RELEASE}" "${FILE}" "0.1.5" "2026-10-08" \
  >"${WORK_DIR}/case1.out" 2>"${WORK_DIR}/case1.err" && RC=0 || RC=$?
assert_eq "case 1 exits 0" "0" "${RC}"
EXPECTED="${WORK_DIR}/case1.expected"
{
  printf '%s\n' "${PREAMBLE}"
  echo
  echo "## [Unreleased]"
  echo
  echo "## [0.1.5] - 2026-10-08"
  echo
  echo "### Added"
  echo
  echo "- A new thing (#90)."
  echo
  echo "### Changed"
  echo
  echo "- A changed thing (#91)."
  echo
  echo "## [0.1.4] - 2026-10-05"
  echo
  echo "### Changed"
  echo
  echo "- Something already released (#70)."
  echo
  printf '%s\n' "${OLDER}"
} >"${EXPECTED}"
assert_file_eq "case 1 releases exactly as expected" "${FILE}" "${EXPECTED}"
assert_contains "case 1 reports the release on stderr" \
  "released [Unreleased] as [0.1.5] - 2026-10-08" "${WORK_DIR}/case1.err"

echo ""
echo "=== 2: Empty Unreleased, no entry -> nothing to do ==="
FILE="${WORK_DIR}/case2.md"
write_fixture "${FILE}" ""
cp "${FILE}" "${WORK_DIR}/case2.before"
bash "${CHANGELOG_RELEASE}" "${FILE}" "0.1.5" "2026-10-08" \
  >"${WORK_DIR}/case2.out" 2>"${WORK_DIR}/case2.err" && RC=0 || RC=$?
assert_eq "case 2 exits 0" "0" "${RC}"
assert_file_eq "case 2 leaves the file byte-identical" "${FILE}" "${WORK_DIR}/case2.before"
assert_contains "case 2 says nothing to release" \
  "[Unreleased] is empty — nothing to release under 0.1.5" "${WORK_DIR}/case2.err"

echo ""
echo "=== 3: Empty Unreleased + entry creates a Changed subsection ==="
FILE="${WORK_DIR}/case3.md"
write_fixture "${FILE}" ""
bash "${CHANGELOG_RELEASE}" "${FILE}" "0.1.5" "2026-10-08" \
  "Weekly Cargo dependency update (#81)." \
  >"${WORK_DIR}/case3.out" 2>"${WORK_DIR}/case3.err" && RC=0 || RC=$?
assert_eq "case 3 exits 0" "0" "${RC}"
EXPECTED="${WORK_DIR}/case3.expected"
{
  printf '%s\n' "${PREAMBLE}"
  echo
  echo "## [Unreleased]"
  echo
  echo "## [0.1.5] - 2026-10-08"
  echo
  echo "### Changed"
  echo
  echo "- Weekly Cargo dependency update (#81)."
  echo
  echo "## [0.1.4] - 2026-10-05"
  echo
  echo "### Changed"
  echo
  echo "- Something already released (#70)."
  echo
  printf '%s\n' "${OLDER}"
} >"${EXPECTED}"
assert_file_eq "case 3 releases with a fresh Changed subsection" "${FILE}" "${EXPECTED}"

echo ""
echo "=== 4: Unreleased has only Added + entry -> Changed added after it ==="
FILE="${WORK_DIR}/case4.md"
write_fixture "${FILE}" "### Added

- A new thing (#90)."
bash "${CHANGELOG_RELEASE}" "${FILE}" "0.1.5" "2026-10-08" \
  "Weekly Cargo dependency update (#81)." \
  >"${WORK_DIR}/case4.out" 2>"${WORK_DIR}/case4.err" && RC=0 || RC=$?
assert_eq "case 4 exits 0" "0" "${RC}"
EXPECTED="${WORK_DIR}/case4.expected"
{
  printf '%s\n' "${PREAMBLE}"
  echo
  echo "## [Unreleased]"
  echo
  echo "## [0.1.5] - 2026-10-08"
  echo
  echo "### Added"
  echo
  echo "- A new thing (#90)."
  echo
  echo "### Changed"
  echo
  echo "- Weekly Cargo dependency update (#81)."
  echo
  echo "## [0.1.4] - 2026-10-05"
  echo
  echo "### Changed"
  echo
  echo "- Something already released (#70)."
  echo
  printf '%s\n' "${OLDER}"
} >"${EXPECTED}"
assert_file_eq "case 4 appends a Changed subsection after Added" "${FILE}" "${EXPECTED}"

echo ""
echo "=== 5: Unreleased has Changed with one line + entry -> appended ==="
FILE="${WORK_DIR}/case5.md"
write_fixture "${FILE}" "### Changed

- An existing change (#92)."
bash "${CHANGELOG_RELEASE}" "${FILE}" "0.1.5" "2026-10-08" \
  "Weekly Cargo dependency update (#81)." \
  >"${WORK_DIR}/case5.out" 2>"${WORK_DIR}/case5.err" && RC=0 || RC=$?
assert_eq "case 5 exits 0" "0" "${RC}"
EXPECTED="${WORK_DIR}/case5.expected"
{
  printf '%s\n' "${PREAMBLE}"
  echo
  echo "## [Unreleased]"
  echo
  echo "## [0.1.5] - 2026-10-08"
  echo
  echo "### Changed"
  echo
  echo "- An existing change (#92)."
  echo "- Weekly Cargo dependency update (#81)."
  echo
  echo "## [0.1.4] - 2026-10-05"
  echo
  echo "### Changed"
  echo
  echo "- Something already released (#70)."
  echo
  printf '%s\n' "${OLDER}"
} >"${EXPECTED}"
assert_file_eq "case 5 appends after the existing Changed line" "${FILE}" "${EXPECTED}"

echo ""
echo "=== 6: Unreleased has Added and Fixed + entry -> Changed inserted between ==="
FILE="${WORK_DIR}/case6.md"
write_fixture "${FILE}" "### Added

- A new thing (#90).

### Fixed

- A fixed thing (#93)."
bash "${CHANGELOG_RELEASE}" "${FILE}" "0.1.5" "2026-10-08" \
  "Weekly Cargo dependency update (#81)." \
  >"${WORK_DIR}/case6.out" 2>"${WORK_DIR}/case6.err" && RC=0 || RC=$?
assert_eq "case 6 exits 0" "0" "${RC}"
EXPECTED="${WORK_DIR}/case6.expected"
{
  printf '%s\n' "${PREAMBLE}"
  echo
  echo "## [Unreleased]"
  echo
  echo "## [0.1.5] - 2026-10-08"
  echo
  echo "### Added"
  echo
  echo "- A new thing (#90)."
  echo
  echo "### Changed"
  echo
  echo "- Weekly Cargo dependency update (#81)."
  echo
  echo "### Fixed"
  echo
  echo "- A fixed thing (#93)."
  echo
  echo "## [0.1.4] - 2026-10-05"
  echo
  echo "### Changed"
  echo
  echo "- Something already released (#70)."
  echo
  printf '%s\n' "${OLDER}"
} >"${EXPECTED}"
assert_file_eq "case 6 inserts Changed between Added and Fixed" "${FILE}" "${EXPECTED}"

echo ""
echo "=== 7: an entry already present under Unreleased is not duplicated ==="
FILE="${WORK_DIR}/case7.md"
write_fixture "${FILE}" "### Changed

- Weekly Cargo dependency update (#81)."
bash "${CHANGELOG_RELEASE}" "${FILE}" "0.1.5" "2026-10-08" \
  "Weekly Cargo dependency update (#81)." \
  >"${WORK_DIR}/case7.out" 2>"${WORK_DIR}/case7.err" && RC=0 || RC=$?
assert_eq "case 7 exits 0" "0" "${RC}"
assert_eq "case 7 has exactly one occurrence of the entry" "1" \
  "$(grep -c -- '- Weekly Cargo dependency update (#81).' "${FILE}")"

echo ""
echo "=== 8: version already released, Unreleased empty, entry given -> no-op ==="
FILE="${WORK_DIR}/case8.md"
write_fixture "${FILE}" ""
# Pre-release 0.1.5 by hand, as though a previous run had already done it.
TMP="${WORK_DIR}/case8.tmp"
{
  head -n 2 "${FILE}"
  echo
  echo "## [0.1.5] - 2026-10-01"
  echo
  echo "### Changed"
  echo
  echo "- Already released separately (#95)."
  echo
  tail -n +3 "${FILE}"
} >"${TMP}"
mv "${TMP}" "${FILE}"
cp "${FILE}" "${WORK_DIR}/case8.before"
bash "${CHANGELOG_RELEASE}" "${FILE}" "0.1.5" "2026-10-08" \
  "Weekly Cargo dependency update (#81)." \
  >"${WORK_DIR}/case8.out" 2>"${WORK_DIR}/case8.err" && RC=0 || RC=$?
assert_eq "case 8 exits 0" "0" "${RC}"
assert_file_eq "case 8 leaves the file byte-identical" "${FILE}" "${WORK_DIR}/case8.before"
assert_contains "case 8 says already released" \
  "[0.1.5] is already released — nothing to do" "${WORK_DIR}/case8.err"

echo ""
echo "=== 9: version already has a heading, Unreleased still holds an entry -> fail ==="
FILE="${WORK_DIR}/case9.md"
write_fixture "${FILE}" "### Added

- Should have been released already (#94)."
# Fake an existing [0.1.5] heading above, mimicking a stale re-run scenario.
TMP="${WORK_DIR}/case9.tmp"
{
  head -n 2 "${FILE}"
  echo
  echo "## [0.1.5] - 2026-10-01"
  echo
  echo "### Changed"
  echo
  echo "- Already released separately (#95)."
  echo
  tail -n +3 "${FILE}"
} >"${TMP}"
mv "${TMP}" "${FILE}"
cp "${FILE}" "${WORK_DIR}/case9.before"
bash "${CHANGELOG_RELEASE}" "${FILE}" "0.1.5" "2026-10-08" \
  >"${WORK_DIR}/case9.out" 2>"${WORK_DIR}/case9.err" && RC=0 || RC=$?
assert_eq "case 9 exits 1" "1" "${RC}"
assert_file_eq "case 9 leaves the file byte-identical" "${FILE}" "${WORK_DIR}/case9.before"
assert_contains "case 9 names the version" "0.1.5" "${WORK_DIR}/case9.err"
assert_contains "case 9 says move them" "move them" "${WORK_DIR}/case9.err"

echo ""
echo "=== 10: releasing then re-running with the same version is idempotent ==="
FILE="${WORK_DIR}/case10.md"
write_fixture "${FILE}" "### Added

- A new thing (#90)."
bash "${CHANGELOG_RELEASE}" "${FILE}" "0.1.5" "2026-10-08" >/dev/null 2>/dev/null
cp "${FILE}" "${WORK_DIR}/case10.after-first"
bash "${CHANGELOG_RELEASE}" "${FILE}" "0.1.5" "2026-10-08" \
  >"${WORK_DIR}/case10.out" 2>"${WORK_DIR}/case10.err" && RC=0 || RC=$?
assert_eq "case 10 second run exits 0" "0" "${RC}"
assert_file_eq "case 10 second run is byte-identical" "${FILE}" "${WORK_DIR}/case10.after-first"

echo ""
echo "=== 11: a changelog with no release yet ends with a single trailing newline ==="
FILE="${WORK_DIR}/case11.md"
{
  printf '%s\n' "${PREAMBLE}"
  echo
  echo "## [Unreleased]"
  echo
  echo "### Added"
  echo
  echo "- The very first entry (#1)."
} >"${FILE}"
bash "${CHANGELOG_RELEASE}" "${FILE}" "0.1.0" "2026-08-31" \
  >"${WORK_DIR}/case11.out" 2>"${WORK_DIR}/case11.err" && RC=0 || RC=$?
assert_eq "case 11 exits 0" "0" "${RC}"
LAST_BYTE="$(tail -c1 "${FILE}" | od -An -c | tr -d ' ')"
assert_eq "case 11 ends with a newline" '\n' "${LAST_BYTE}"
SECOND_LAST_BYTE="$(tail -c2 "${FILE}" | head -c1 | od -An -c | tr -d ' ')"
assert_eq "case 11 has exactly one trailing newline, not two" "." "${SECOND_LAST_BYTE}"

echo ""
echo "=== 12: failures exit 1 and leave the file unchanged ==="

FILE="${WORK_DIR}/case12-nounrel.md"
{
  printf '%s\n' "${PREAMBLE}"
  echo
  echo "## [0.1.0] - 2026-08-31"
} >"${FILE}"
cp "${FILE}" "${WORK_DIR}/case12-nounrel.before"
bash "${CHANGELOG_RELEASE}" "${FILE}" "0.1.1" "2026-10-08" \
  >/dev/null 2>"${WORK_DIR}/case12-nounrel.err" && RC=0 || RC=$?
assert_eq "missing Unreleased heading exits 1" "1" "${RC}"
assert_file_eq "missing Unreleased heading leaves the file alone" \
  "${FILE}" "${WORK_DIR}/case12-nounrel.before"
assert_contains "missing Unreleased heading is named" \
  "no \`## [Unreleased]\` heading" "${WORK_DIR}/case12-nounrel.err"

bash "${CHANGELOG_RELEASE}" "${WORK_DIR}/absent.md" "0.1.1" "2026-10-08" \
  >/dev/null 2>"${WORK_DIR}/case12-missing.err" && RC=0 || RC=$?
assert_eq "missing file exits 1" "1" "${RC}"
assert_contains "missing file is named" "no such changelog" "${WORK_DIR}/case12-missing.err"

FILE="${WORK_DIR}/case12-badver.md"
write_fixture "${FILE}" ""
cp "${FILE}" "${WORK_DIR}/case12-badver.before"
bash "${CHANGELOG_RELEASE}" "${FILE}" "0.1" "2026-10-08" \
  >/dev/null 2>"${WORK_DIR}/case12-badver.err" && RC=0 || RC=$?
assert_eq "malformed version exits 1" "1" "${RC}"
assert_file_eq "malformed version leaves the file alone" \
  "${FILE}" "${WORK_DIR}/case12-badver.before"
assert_contains "malformed version is named" "malformed version" "${WORK_DIR}/case12-badver.err"

FILE="${WORK_DIR}/case12-baddate.md"
write_fixture "${FILE}" ""
cp "${FILE}" "${WORK_DIR}/case12-baddate.before"
bash "${CHANGELOG_RELEASE}" "${FILE}" "0.1.5" "2026/10/08" \
  >/dev/null 2>"${WORK_DIR}/case12-baddate.err" && RC=0 || RC=$?
assert_eq "malformed date exits 1" "1" "${RC}"
assert_file_eq "malformed date leaves the file alone" \
  "${FILE}" "${WORK_DIR}/case12-baddate.before"
assert_contains "malformed date is named" "malformed date" "${WORK_DIR}/case12-baddate.err"

FILE="${WORK_DIR}/case12-multiline.md"
write_fixture "${FILE}" ""
cp "${FILE}" "${WORK_DIR}/case12-multiline.before"
bash "${CHANGELOG_RELEASE}" "${FILE}" "0.1.5" "2026-10-08" "$(printf 'one\ntwo')" \
  >/dev/null 2>"${WORK_DIR}/case12-multiline.err" && RC=0 || RC=$?
assert_eq "multiline entry exits 1" "1" "${RC}"
assert_file_eq "multiline entry leaves the file alone" \
  "${FILE}" "${WORK_DIR}/case12-multiline.before"
assert_contains "multiline entry is named" "entry must be a single line" \
  "${WORK_DIR}/case12-multiline.err"

bash "${CHANGELOG_RELEASE}" >/dev/null 2>"${WORK_DIR}/case12-usage.err" && RC=0 || RC=$?
assert_eq "no arguments exits 1" "1" "${RC}"
assert_contains "no arguments prints usage" "usage: changelog-release.sh" \
  "${WORK_DIR}/case12-usage.err"

echo ""
echo "=== 13: an entry with an embedded backslash is written literally ==="
FILE="${WORK_DIR}/case13.md"
write_fixture "${FILE}" ""
bash "${CHANGELOG_RELEASE}" "${FILE}" "0.1.5" "2026-10-08" \
  'Path a\b (#9).' \
  >"${WORK_DIR}/case13.out" 2>"${WORK_DIR}/case13.err" && RC=0 || RC=$?
assert_eq "case 13 exits 0" "0" "${RC}"
assert_contains "case 13 writes the backslash literally" \
  '- Path a\b (#9).' "${FILE}"

echo ""
echo "=== summary: ${PASSED} passed, ${FAILED} failed ==="
[[ "${FAILED}" -eq 0 ]]
