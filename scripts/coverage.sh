#!/usr/bin/env bash
# Line coverage of the compiler and its crates under every suite the gate
# runs — the unit tests, the docs check, the parity corpus,
# the served pages, the book, twenty generated programs, the layout and
# module tests, the harness programs in both modes, the parts and the task
# board — measured by cargo llvm-cov (design rule 22). Fails when a crate
# falls under its floor, or when a unit test fails in its build: the one
# run of the unit tests in a release build (CI's test job runs them in a
# debug one). Usage: scripts/coverage.sh [--quick] [--arms]
#   --quick: the unit tests alone, no floors.
#   --arms:  also write target/coverage/{coverage.json,missing.txt,arms.txt}:
#            the lines no suite reaches, by file, the functions that never
#            ran, the rows no suite calls, and the match arms among the
#            uncovered lines of the backends (scripts/coverage_arms.py) —
#            where a lowering slip hides.
#   The floors are the gate's measurement of 2026-10-08 under every suite —
#   crow 83.35%, crow_runtime 88.93%, crow_web 76.56%, crow_db 84.16% —
#   rounded down by one; they only rise. A change that lowers one names the uncovered lines
#   in its commit message, with why they cannot be reached.
# Written for the bash every Mac ships (3.2) and the CI runner's: no
# associative arrays, a temporary file named the way both mktemps take.
set -e
cd "$(dirname "$0")/.."
command -v cargo-llvm-cov > /dev/null || { echo "cargo-llvm-cov is not installed: cargo install cargo-llvm-cov --locked (and rustup component add llvm-tools-preview)"; exit 2; }
quick=0; arms=0
for a in "$@"; do
  case $a in
    --quick) quick=1 ;;
    --arms) arms=1 ;;
    *) echo "usage: scripts/coverage.sh [--quick] [--arms]"; exit 2 ;;
  esac
done
floor_of() {
  case $1 in
    crow) echo 82 ;;
    crow_runtime) echo 87 ;;
    crow_web) echo 75 ;;
    crow_db) echo 83 ;;
  esac
}
export CROW_IR_VERIFY=strict
# (only the profiles go: the instrumented build is kept between runs, its
# flags never change, so a run after the first rebuilds what changed — and
# the executables go with them: the report reads every executable under
# the target directory, so a test binary a build of 2026-10-17 had left
# beside the day's, its functions matching no profile, pulled the
# compiler's coverage from 82% to 50% while every test passed; the build
# relinks the day's binaries from the objects it kept)
cargo llvm-cov clean --workspace --profraw-only > /dev/null 2>&1
find target/llvm-cov-target/release/deps -maxdepth 1 -type f -perm -100 ! -name '*.dylib' ! -name '*.so' -delete 2> /dev/null || true
rm -f target/llvm-cov-target/release/crow
# Every crate's tests run, past a test that fails: cargo stops at the first
# failing test binary otherwise, and the crates after it went unmeasured —
# their floors failed in place of the test (2026-11-09: a web test, then
# crow_runtime, crow_web and crow_db each 25 points under), and a test
# failing in the last crate's binary would have passed with its floor. A
# test that fails in this build (instrumented, release) fails the run, shown
# with what it said; the whole output is target/coverage/tests.txt.
mkdir -p target/coverage
tests=target/coverage/tests.txt
set +e
cargo llvm-cov --workspace --release --no-report --no-fail-fast 2>&1 | tee "$tests" | grep -E '^test result|^error'
tested=${PIPESTATUS[0]}
set -e
if [ "$tested" != 0 ]; then
  awk '/^failures:$/ { on = 1 } on { print } /^test result: FAILED/ { on = 0 }' "$tests"
  echo "✗ the tests failed in the instrumented build (exit $tested): target/coverage/tests.txt has their whole output"
fi
if [ $quick = 0 ]; then
  # the instrumented binary, run as the gate runs it; the programs it
  # compiles link an instrumented runtime, so both modes count
  run() { cargo llvm-cov run --release --no-report --bin crow -- "$@" 2>&1 | tail -1; }
  for suite in --docs --errors --parity --served; do run test $suite; done
  CROW_BOOK_JOBS=2 run test --book
  run test --fuzz 20
  run test --fuzz 50 --mistakes
  for f in tests/layout/conformance.co examples/layout_dashboard.co src/modules/*.co; do run test $f; done
  for f in tests/*_harness.co; do run test $f; run test --compiled $f; done
  # the suites that run from their own directory: the binary itself, its
  # profile written where the report reads
  crow=$PWD/target/llvm-cov-target/release/crow
  export LLVM_PROFILE_FILE=$PWD/target/llvm-cov-target/crow-extra-%p-%16m.profraw
  (cd tests/parts && "$crow" test views/sim.co && "$crow" test sim/clock.co) 2>&1 | tail -1
  (cd examples/taskboard && "$crow" test board/door.co && "$crow" test board/cards.co && "$crow" test board/page.co) 2>&1 | tail -1
  unset LLVM_PROFILE_FILE
fi
report=$(mktemp "${TMPDIR:-/tmp}/crow_coverage.XXXXXX")
cargo llvm-cov report --failure-mode all --release --workspace > "$report"
tail -1 "$report" | awk '{ printf "workspace: %s lines, %s covered\n", $8, $10 }'
fail=0
for crate in crow crow_runtime crow_web crow_db; do
  pct=$(awk -v c="$crate" '
    $1 == "Filename" || $1 == "TOTAL" || $1 ~ /^-/ { next }
    { which = ($1 ~ /^crow_runtime\//) ? "crow_runtime" : ($1 ~ /^crow_web\//) ? "crow_web" : ($1 ~ /^crow_db\//) ? "crow_db" : "crow";
      if (which == c) { lines += $8; missed += $9 } }
    END { if (lines) printf "%.2f", 100 * (lines - missed) / lines; else print "0" }' "$report")
  floor=$(floor_of "$crate")
  if [ $quick = 1 ]; then
    printf '  %-13s %6s%% (the unit tests alone; no floor)\n' "$crate" "$pct"
    continue
  fi
  mark="✓"
  if awk -v a="$pct" -v f="$floor" 'BEGIN { exit !(a + 0 < f + 0) }'; then mark="✗"; fail=1; fi
  printf '%s %-13s %6s%% (floor %s)\n' "$mark" "$crate" "$pct" "$floor"
done
rm -f "$report"
if [ $arms = 1 ]; then
  mkdir -p target/coverage
  cargo llvm-cov report --failure-mode all --release --workspace --json --output-path target/coverage/coverage.json > /dev/null 2>&1
  cargo llvm-cov report --failure-mode all --release --workspace --show-missing-lines > target/coverage/missing.txt 2> /dev/null
  python3 scripts/coverage_arms.py target/coverage/coverage.json target/coverage/missing.txt > target/coverage/arms.txt
  echo "arms: target/coverage/arms.txt ($(grep -c '' target/coverage/arms.txt) lines)"
fi
[ "$tested" = 0 ] || fail=1
exit $fail
