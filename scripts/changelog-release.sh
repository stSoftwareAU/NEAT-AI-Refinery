#!/usr/bin/env bash
# changelog-release.sh — release CHANGELOG.md's `[Unreleased]` section under
# the crate version a PR ships, so the next bump cannot mislabel those
# entries as belonging to a later release (Issue #80).
#
# `scripts/auto-version.sh` bumps `refinery/Cargo.toml`'s patch, but on its
# own leaves `## [Unreleased]` unreleased. Left alone, the next bump labels
# those entries with a later version than the one that shipped them, and the
# shipping version gets no heading at all. This script turns `[Unreleased]`
# into `## [<version>] - <date>` the moment that version is settled, and
# optionally records a single entry first — the weekly Cargo refresh PR's
# "Weekly Cargo dependency update (#<PR>)." line.
#
# Usage:
#   changelog-release.sh <changelog> <version> <date> [entry]
#
#     <changelog>  path to CHANGELOG.md
#     <version>    the version to release, x.y.z (no leading v)
#     <date>       the release date, YYYY-MM-DD
#     [entry]      an optional single-line note to record under
#                   `### Changed` before releasing; an empty string means no
#                   entry
#
# An empty [Unreleased] is never released — nothing is lost, the
# file is simply left alone (exit 0). A version that already has its own
# `## [<version>] - ` heading is left alone too, unless [Unreleased] still
# holds entries, in which case the run fails loud rather than silently
# dropping them.

set -euo pipefail

die() {
  echo "changelog-release.sh: $1" >&2
  exit 1
}

[ "$#" -eq 3 ] || [ "$#" -eq 4 ] \
  || die "usage: changelog-release.sh <changelog> <version> <date> [entry]"

changelog="$1"
version="$2"
date_arg="$3"
entry="${4:-}"

[ -f "$changelog" ] || die "no such changelog: $changelog"

printf '%s' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' \
  || die "malformed version (expected x.y.z): $version"

printf '%s' "$date_arg" | grep -Eq '^[0-9]{4}-[0-9]{2}-[0-9]{2}$' \
  || die "malformed date (expected YYYY-MM-DD): $date_arg"

case "$entry" in
  *$'\n'* | *$'\r'*)
    die "entry must be a single line"
    ;;
esac

tmp_out="$(mktemp)"
status_file="$(mktemp)"
trap 'rm -f "$tmp_out" "$status_file"' EXIT

export CR_VERSION="$version"
export CR_DATE="$date_arg"
export CR_ENTRY="$entry"
export CR_PATH="$changelog"
export CR_STATUS_FILE="$status_file"

awk '
function entry_line() {
  return "- " ENVIRON["CR_ENTRY"]
}

function is_entry(line) {
  if (line == "") return 0
  if (index(line, "### ") == 1) return 0
  if (index(line, "<!--") == 1) return 0
  return 1
}

{
  lines[NR] = $0
}

END {
  n = NR
  version = ENVIRON["CR_VERSION"]
  date = ENVIRON["CR_DATE"]
  entry = ENVIRON["CR_ENTRY"]
  has_entry_arg = (entry != "")
  path = ENVIRON["CR_PATH"]
  status_file = ENVIRON["CR_STATUS_FILE"]

  heading_marker = "## [" version "] - "

  u = 0
  for (i = 1; i <= n; i++) {
    if (lines[i] == "## [Unreleased]") { u = i; break }
  }
  if (u == 0) {
    print "changelog-release.sh: no `## [Unreleased]` heading in " path > "/dev/stderr"
    print "die" > status_file
    close(status_file)
    exit 0
  }

  body_start = u + 1
  body_end = n
  next_heading = 0
  for (i = body_start; i <= n; i++) {
    if (index(lines[i], "## ") == 1) { body_end = i - 1; next_heading = i; break }
  }

  # Determine whether this version already has a heading.
  already = 0
  for (i = 1; i <= n; i++) {
    if (index(lines[i], heading_marker) == 1) { already = 1; break }
  }

  body_has_entry = 0
  for (i = body_start; i <= body_end; i++) {
    if (is_entry(lines[i])) { body_has_entry = 1; break }
  }

  if (already) {
    if (body_has_entry) {
      print "changelog-release.sh: " path " already has a [" version "] section, but [Unreleased] still holds entries — move them under [" version "]" > "/dev/stderr"
      print "die" > status_file
      close(status_file)
      exit 0
    } else {
      print "changelog-release.sh: [" version "] is already released — nothing to do" > "/dev/stderr"
      print "noop" > status_file
      close(status_file)
      exit 0
    }
  }

  # Build the (possibly modified) body as an array of lines.
  delete newbody
  bn = 0
  for (i = body_start; i <= body_end; i++) {
    bn++
    newbody[bn] = lines[i]
  }

  if (has_entry_arg) {
    wanted = entry_line()
    already_present = 0
    for (i = 1; i <= bn; i++) {
      if (newbody[i] == wanted) { already_present = 1; break }
    }
    if (!already_present) {
      # Find a "### Changed" line.
      changed_idx = 0
      for (i = 1; i <= bn; i++) {
        if (newbody[i] == "### Changed") { changed_idx = i; break }
      }
      if (changed_idx > 0) {
        # End of this subsection: next "### " line or end of body.
        end_idx = bn
        for (i = changed_idx + 1; i <= bn; i++) {
          if (index(newbody[i], "### ") == 1) { end_idx = i - 1; break }
        }
        # Last non-blank line within [changed_idx, end_idx].
        last_nonblank = changed_idx
        for (i = changed_idx; i <= end_idx; i++) {
          if (newbody[i] != "") last_nonblank = i
        }
        delete rebuilt
        rn = 0
        for (i = 1; i <= last_nonblank; i++) { rn++; rebuilt[rn] = newbody[i] }
        rn++; rebuilt[rn] = wanted
        for (i = last_nonblank + 1; i <= bn; i++) { rn++; rebuilt[rn] = newbody[i] }
        delete newbody
        for (i = 1; i <= rn; i++) { newbody[i] = rebuilt[i] }
        bn = rn
      } else {
        # Insert a fresh "### Changed" subsection before the first of
        # Deprecated/Removed/Fixed/Security, else at the end of the body.
        insert_before = 0
        for (i = 1; i <= bn; i++) {
          if (newbody[i] == "### Deprecated" || newbody[i] == "### Removed" || \
              newbody[i] == "### Fixed" || newbody[i] == "### Security") {
            insert_before = i
            break
          }
        }
        delete rebuilt
        rn = 0
        if (insert_before > 0) {
          for (i = 1; i <= insert_before - 1; i++) { rn++; rebuilt[rn] = newbody[i] }
          # Ensure one blank line separates the previous subsection, unless
          # the body was empty up to here.
          if (rn > 0 && rebuilt[rn] != "") { rn++; rebuilt[rn] = "" }
          rn++; rebuilt[rn] = "### Changed"
          rn++; rebuilt[rn] = ""
          rn++; rebuilt[rn] = wanted
          rn++; rebuilt[rn] = ""
          for (i = insert_before; i <= bn; i++) { rn++; rebuilt[rn] = newbody[i] }
        } else {
          for (i = 1; i <= bn; i++) { rn++; rebuilt[rn] = newbody[i] }
          if (rn > 0 && rebuilt[rn] != "") { rn++; rebuilt[rn] = "" }
          rn++; rebuilt[rn] = "### Changed"
          rn++; rebuilt[rn] = ""
          rn++; rebuilt[rn] = wanted
        }
        delete newbody
        for (i = 1; i <= rn; i++) { newbody[i] = rebuilt[i] }
        bn = rn
      }
    }
  }

  # Recompute whether the (possibly-modified) body has an entry.
  body_has_entry = 0
  for (i = 1; i <= bn; i++) {
    if (is_entry(newbody[i])) { body_has_entry = 1; break }
  }

  if (!body_has_entry) {
    print "changelog-release.sh: [Unreleased] is empty — nothing to release under " version > "/dev/stderr"
    print "noop" > status_file
    close(status_file)
    exit 0
  }

  # Strip leading and trailing blank lines from the body.
  lo = 1
  while (lo <= bn && newbody[lo] == "") lo++
  hi = bn
  while (hi >= lo && newbody[hi] == "") hi--

  # Emit everything before U unchanged.
  for (i = 1; i < u; i++) print lines[i]
  print "## [Unreleased]"
  print ""
  print "## [" version "] - " date
  print ""
  for (i = lo; i <= hi; i++) print newbody[i]

  if (next_heading > 0) {
    print ""
    for (i = next_heading; i <= n; i++) print lines[i]
  }

  print "changelog-release.sh: released [Unreleased] as [" version "] - " date > "/dev/stderr"
  print "release" > status_file
  close(status_file)
}
' "$changelog" >"$tmp_out"

status="$(cat "$status_file")"

case "$status" in
  die)
    exit 1
    ;;
  noop)
    exit 0
    ;;
  release)
    mv "$tmp_out" "$changelog"
    exit 0
    ;;
  *)
    die "internal error: no outcome recorded for $changelog"
    ;;
esac
