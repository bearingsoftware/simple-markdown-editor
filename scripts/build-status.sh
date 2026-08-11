#!/usr/bin/env bash
# Build status.json and the coverage bundle the status board ingests (MDEDIT-83).
#
# Reads the profdata that `cargo llvm-cov --no-report` has already produced, so
# the tests run once and both reports come from the same measurement.
#
# Outputs:
#   status.json               unit coverage, merged + per-crate, and the floors
#   coverage-lcov/unit/       lcov.info + the sources it references
#
# Usage: scripts/build-status.sh <commit-sha>

set -euo pipefail

project="simple-markdown-editor"
commit="${1:?commit sha required}"

root="$(cd "$(dirname "$0")/.." && pwd)"

# The floors the board judges a regression against (MDEDIT-130).
#
# **Committed, never computed.** A floor derived from the run being measured
# always equals the actual and can never be breached, which is a floor in name
# only. Changing one is a commit somebody reads: raising it after coverage
# improves is the ratchet, and lowering it has to be visible in a diff.
#
# **menu-win and shell-win carry no floor on purpose.** Coverage is measured on
# a Linux runner, where nearly all of both is cfg'd out — menu-win's 14.3% and
# shell-win's 94.3% describe the Linux slice, not the crate, and either would
# move on a cfg change with no test touched. Flooring them needs a Windows
# coverage run, which is its own ticket.
floors="$root/scripts/coverage-floors.json"

out="$root/coverage-lcov"
rm -rf "$out"
mkdir -p "$out/unit"

cargo llvm-cov report --lcov --output-path "$out/unit/lcov.info"
cargo llvm-cov report --json --output-path "$root/coverage.json"

# The board joins sources by the SF path, so they have to be repo-relative.
# llvm-cov writes absolute ones.
sed -i "s#^SF:$root/#SF:#" "$out/unit/lcov.info"

# Merged line coverage, and a shard per crate. Crates whose lines never got
# compiled on this platform (the Windows-only ones) have no regions to measure,
# so they are dropped rather than reported as a misleading zero.
jq --arg project "$project" --arg commit "$commit" --arg now "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
   --slurpfile floors "$floors" '
  .data[0] as $d
  | ($d.files
     | map(select(.filename | test("/crates/[^/]+/src/")))
     | map({ crate: (.filename | capture("/crates/(?<c>[^/]+)/src/").c),
             count: .summary.lines.count, covered: .summary.lines.covered })
     | group_by(.crate)
     | map(select((map(.count) | add) > 0))
     | map({ (.[0].crate): (((map(.covered) | add) / (map(.count) | add) * 1000 | round) / 10) })
     | add // {}) as $shards
  | { project: $project,
      commit: $commit,
      fetched_at: $now,
      coverage: { unit: { merged: (($d.totals.lines.percent * 10 | round) / 10),
                          shards: $shards },
                  floors: $floors[0] } }
' "$root/coverage.json" > "$root/status.json"

# Copy every source the report references into the bundle so the board can show
# annotated lines, not just percentages.
missing=0
while IFS= read -r sf; do
  if [ -f "$root/$sf" ]; then
    mkdir -p "$out/unit/$(dirname "$sf")"
    cp "$root/$sf" "$out/unit/$sf"
  else
    missing=$((missing + 1))
  fi
done < <(grep '^SF:' "$out/unit/lcov.info" | sed 's/^SF://' | sort -u)

echo "merged: $(jq -r '.coverage.unit.merged' "$root/status.json")%" >&2
echo "shards: $(jq -r '.coverage.unit.shards | keys | join(", ")' "$root/status.json")" >&2
echo "bundle: $(grep -c '^SF:' "$out/unit/lcov.info") files, $missing source(s) not found" >&2

# What the board is about to conclude, said here too, so a local run does not
# have to wait for a red dot to find out. A floor whose crate produced no
# regions has no actual to compare and is inert rather than a false breach.
breaches="$(jq -r '
  .coverage as $c
  | ($c.unit.shards + { merged: $c.unit.merged }) as $actual
  | $c.floors.unit
  | to_entries
  | map(select($actual[.key] != null and $actual[.key] < .value)
        | "\(.key) \($actual[.key])% under floor \(.value)%")
  | join("; ")
' "$root/status.json")"
if [ -n "$breaches" ]; then
  echo "floors: BREACHED — $breaches" >&2
else
  echo "floors: $(jq -r '.coverage.floors.unit | length' "$root/status.json") declared, none breached" >&2
fi
