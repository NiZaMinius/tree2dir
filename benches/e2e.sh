#!/usr/bin/env bash
# End-to-end timing with hyperfine: whole process, output discarded.
#
#   1. build into a new directory: tree2dir against plain `mkdir -p` and `: >`
#      for the same structure;
#   2. dry run on a tree that is already built;
#   3. second run on a built tree (every file exists and is skipped).
#
# Needs `hyperfine` and a release build (`cargo build --release`).
# Usage: benches/e2e.sh [N]   (default 1000 entries)
set -euo pipefail
n=${1:-1000}
here=$(cd "$(dirname "$0")" && pwd)
bin="$here/../target/release/tree2dir"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

"$here/gen_tree.sh" "$n" > "$work/tree.txt"

# Shell baseline for the identical structure.
{
  echo '#!/usr/bin/env bash'
  echo 'out=$1; mkdir -p "$out"'
  awk '/├── dir/ { sub(/^├── /, ""); sub(/\/$/, ""); d = $0; print "mkdir -p \"$out/" d "\"" }
       /│   ├── / { sub(/^│   ├── /, ""); print ": > \"$out/" d "/" $0 "\"" }' "$work/tree.txt"
} > "$work/baseline.sh"
chmod +x "$work/baseline.sh"

echo "### 1. build into a new directory ($n entries)"
hyperfine --warmup 3 --prepare "rm -rf $work/out" \
  "$bin --file $work/tree.txt --output $work/out" \
  "$work/baseline.sh $work/out"

"$bin" --file "$work/tree.txt" --output "$work/built" > /dev/null

echo "### 2. dry run on a built tree"
hyperfine --warmup 3 "$bin --file $work/tree.txt --output $work/built --dry-run"

echo "### 3. second run, every file exists"
hyperfine --warmup 3 "$bin --file $work/tree.txt --output $work/built"
