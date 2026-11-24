#!/usr/bin/env bash
# Print an ASCII tree of about N entries (directories of ten files) to stdout.
# Usage: benches/gen_tree.sh 1000 > tree.txt
set -euo pipefail
n=${1:?usage: gen_tree.sh N}
awk -v n="$n" 'BEGIN {
  c = 0; d = 0
  while (c < n) {
    print "├── dir" d "/"; c++
    for (f = 0; f < 10 && c < n; f++) { print "│   ├── file" f ".rs"; c++ }
    d++
  }
}'
