#!/bin/zsh
# Line coverage of the compiler and its crates under the tests the gate
# runs — the unit tests, the docs check, the parity corpus, the packages
# and the served pages — measured by cargo llvm-cov (design rule 22). Fails when a
# crate falls under its floor. Usage: scripts/coverage.sh [--quick]
# (--quick: the unit tests alone, no floors).
#   The floors are the gate's measurement of 2026-10-07 — crow 81.45%,
#   crow_runtime 87.84%, crow_web 76.21%, crow_db 83.85% — rounded down by
#   one; they only rise. A change that lowers one names the uncovered lines
#   in its commit message, with why they cannot be reached. (The book and
#   the harness suites are not under this measurement yet.)
set -e
cd "$(dirname "$0")/.."
command -v cargo-llvm-cov > /dev/null || { echo "cargo-llvm-cov is not installed: cargo install cargo-llvm-cov --locked (and rustup component add llvm-tools-preview)"; exit 2; }
typeset -A FLOOR
FLOOR[crow]=80
FLOOR[crow_runtime]=86
FLOOR[crow_web]=75
FLOOR[crow_db]=82
cargo llvm-cov clean --workspace > /dev/null 2>&1
cargo llvm-cov --workspace --release --no-report 2>&1 | grep -E '^test result|^error' || true
if [ "$1" != "--quick" ]; then
  for suite in --docs --parity --packages --served; do
    cargo llvm-cov run --release --no-report --bin crow -- test $suite 2>&1 | tail -1
  done
fi
report=$(mktemp -t crow_coverage)
cargo llvm-cov report --release --workspace > "$report"
tail -1 "$report" | awk '{ printf "workspace: %s lines, %s covered\n", $8, $10 }'
fail=0
for crate in crow crow_runtime crow_web crow_db; do
  pct=$(awk -v c="$crate" '
    $1 == "Filename" || $1 == "TOTAL" || $1 ~ /^-/ { next }
    { which = ($1 ~ /^crow_runtime\//) ? "crow_runtime" : ($1 ~ /^crow_web\//) ? "crow_web" : ($1 ~ /^crow_db\//) ? "crow_db" : "crow";
      if (which == c) { lines += $8; missed += $9 } }
    END { if (lines) printf "%.2f", 100 * (lines - missed) / lines; else print "0" }' "$report")
  floor=${FLOOR[$crate]}
  mark="✓"
  if awk -v a="$pct" -v f="$floor" 'BEGIN { exit !(a + 0 < f + 0) }'; then mark="✗"; fail=1; fi
  printf '%s %-13s %6s%% (floor %s)\n' "$mark" "$crate" "$pct" "$floor"
  [ "$1" = "--quick" ] && fail=0
done
rm -f "$report"
exit $fail
