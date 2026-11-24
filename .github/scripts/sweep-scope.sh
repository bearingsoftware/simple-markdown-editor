#!/usr/bin/env bash
# Prints the cargo-mutants scope arguments for one sweep shard, one argument per line (MDEDIT-217).
# Usage: sweep-scope.sh SLICE SHARD SHARDS. SHARDS is how many shards a light slice runs (the plan sizes it);
# a heavy slice always has MUTANTS_MAX_SHARDS. Read by `plan` (to count) and `mutate` (to run), so the two
# cannot disagree. Needs MUTANTS_SWEEP_RUNS, MUTANTS_MAX_SHARDS, MUTANTS_HEAVY_FILE,
# MUTANTS_HEAVY_SLICES and MUTANTS_EXCLUDES from the workflow env.
set -euo pipefail

slice=$1
shard=$2
shards=$3
light=$((MUTANTS_SWEEP_RUNS - MUTANTS_HEAVY_SLICES))

# The last HEAVY_SLICES slices are the one big file, split round-robin across all their shards.
if [ "$slice" -ge "$light" ]; then
  parts=$((MUTANTS_HEAVY_SLICES * MUTANTS_MAX_SHARDS))
  part=$(((slice - light) * MUTANTS_MAX_SHARDS + shard))
  printf '%s\n' --file "$MUTANTS_HEAVY_FILE" --shard "$part/$parts"
  exit 0
fi

# Unquoted on purpose: MUTANTS_EXCLUDES is several arguments.
files=$(cargo mutants --list-files $MUTANTS_EXCLUDES | tr -d '\r')
found=0
while IFS= read -r f; do
  [ "$f" = "$MUTANTS_HEAVY_FILE" ] && continue
  # cksum is POSIX and fixed, so every runner agrees. The path is repo-relative with `/`.
  h=$(printf '%s' "$f" | cksum | cut -d' ' -f1)
  if [ $((h % light)) -eq "$slice" ]; then
    printf '%s\n' --file "$f"
    found=$((found + 1))
  fi
done <<< "$files"

if [ "$found" -eq 0 ]; then
  echo "no source file hashes to sweep slice $slice of $light" >&2
  exit 1
fi
printf '%s\n' --shard "$shard/$shards"
