#!/usr/bin/env bash
# Diff two CycloneDX SBOMs and annotate every component that is new (Issue #65).
#
# usage: sbom-diff.sh <previous.cdx.json> <current.cdx.json> <Cargo.toml>
#
# A component is new when its package URL, minus version and qualifiers, is
# absent from the previous SBOM — a version bump of a known crate is not new.
# Each new component is printed as a GitHub Actions annotation:
#   ::notice::   it is a direct dependency declared in <Cargo.toml>;
#   ::warning::  it is not, so something pulled it in transitively.
# The warning is the supply-chain signal: a crate nobody asked for by name.
#
# Exit codes: 0 diffed (whatever it found), 2 bad arguments or unreadable input.
set -euo pipefail

usage() {
  echo "usage: sbom-diff.sh <previous.cdx.json> <current.cdx.json> <Cargo.toml>" >&2
  exit 2
}

[[ $# -eq 3 ]] || usage
PREVIOUS="$1"
CURRENT="$2"
MANIFEST="$3"

for sbom in "${PREVIOUS}" "${CURRENT}"; do
  if [[ ! -f "${sbom}" ]]; then
    echo "sbom-diff: no such SBOM: ${sbom}" >&2
    exit 2
  fi
  if ! jq -e '.bomFormat == "CycloneDX"' "${sbom}" >/dev/null 2>&1; then
    echo "sbom-diff: not a CycloneDX JSON document: ${sbom}" >&2
    exit 2
  fi
done
if [[ ! -f "${MANIFEST}" ]]; then
  echo "sbom-diff: no such manifest: ${MANIFEST}" >&2
  exit 2
fi

# Every crate name the manifest depends on directly, one per line, across
# [dependencies], [dev-dependencies], [build-dependencies], their
# [target.<cfg>.*] forms and the [dependencies.<name>] table form. A
# `package = "…"` rename resolves to the real crate name, which is what the
# SBOM records. POSIX awk only, so it runs on macOS as well as Linux.
direct_dependencies() {
  awk '
    function flush() {
      if (table != "") print table
      table = ""
    }
    function strip(s) {
      gsub(/^[ \t"]+|[ \t"]+$/, "", s)
      return s
    }
    function renamed(line,    s) {
      s = line
      if (sub(/.*package[ \t]*=[ \t]*"/, "", s) == 0) return ""
      sub(/".*/, "", s)
      return s
    }
    /^[ \t]*\[/ {
      flush()
      header = $0
      gsub(/[ \t]/, "", header)
      in_deps = 0
      if (header ~ /^\[(target\..*\.)?(dev-|build-)?dependencies\]$/) {
        in_deps = 1
      } else if (header ~ /^\[(target\..*\.)?(dev-|build-)?dependencies\.[^.]+\]$/) {
        table = header
        sub(/^.*dependencies\./, "", table)
        sub(/\]$/, "", table)
        table = strip(table)
      }
      next
    }
    table != "" && /^[ \t]*package[ \t]*=/ {
      table = renamed($0)
      next
    }
    in_deps && /^[ \t]*"?[A-Za-z0-9_-]+"?[ \t]*(\.[A-Za-z]+[ \t]*)?=/ {
      name = renamed($0)
      if (name == "") {
        name = $0
        sub(/[ \t]*(\.[A-Za-z]+[ \t]*)?=.*/, "", name)
        name = strip(name)
      }
      print name
    }
    END { flush() }
  ' "${MANIFEST}"
}

DIRECT="$(direct_dependencies)"

jq -n -r \
  --slurpfile previous "${PREVIOUS}" \
  --slurpfile current "${CURRENT}" \
  --arg direct "${DIRECT}" \
  --arg manifest "${MANIFEST}" '
  # Crate names compare case-insensitively with "-" and "_" interchangeable.
  def canonical: ascii_downcase | gsub("_"; "-");
  def purls: [.components // [] | .. | objects | .purl? // empty | strings];
  # Identity of a component: its purl without @version, ?qualifiers, #subpath.
  def key: sub("[@?#].*$"; "");
  def crate: key | split("/") | last | canonical;
  # GitHub workflow-command data escaping, so SBOM text cannot open a command.
  def escape: gsub("%"; "%25") | gsub("\r"; "%0D") | gsub("\n"; "%0A");

  ($direct | split("\n") | map(select(length > 0) | canonical)) as $direct_names
  | ($previous[0] | purls | map(key)) as $known
  | [$current[0] | purls | unique[] | select(key as $k | $known | index([$k]) | not)] as $new
  | [$new[] | select(crate as $c | $direct_names | index([$c]) | not)] as $transitive
  | ($new[]
      | if (crate as $c | $direct_names | index([$c])) then
          "::notice title=New direct dependency::"
            + ("\(.) is new since the baseline SBOM and is declared in \($manifest)" | escape)
        else
          "::warning title=New transitive SBOM component::"
            + ("\(.) is new since the baseline SBOM and is not a direct dependency in \($manifest) — run `cargo tree -i \(crate)` to see what pulled it in" | escape)
        end),
    "sbom-diff: \($new | length) new component(s), \($transitive | length) not a direct dependency of \($manifest)"
'
