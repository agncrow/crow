#!/bin/bash
# Every Rosetta Code program in this directory, run in both modes and held
# to its recorded output (<slug>.out); a program with <slug>.in reads it as
# its standard input. Bash as macOS ships it (3.2).
#
#   rosetta/run.sh              every program, interpreted and compiled
#   rosetta/run.sh -i           interpreted only
#   rosetta/run.sh -c           compiled only
#   rosetta/run.sh word ...     only the programs whose names hold a word
#
# CROW names the compiler (default: target/release/crow of this checkout),
# LIMIT the seconds a run may take (default 120). Programs run in
# target/rosetta/ (WORK names another: runs in parallel need one each),
# where the files of rosetta/data/ are linked, since a compiled program
# builds in .crow_build under the current directory and a program reads
# its data from where it runs.
set -u
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
crow=${CROW:-$root/target/release/crow}
limit=${LIMIT:-120}
work=${WORK:-$root/target/rosetta}
modes="interpreted compiled"
words=""
for a in "$@"; do
    case "$a" in
        -i) modes="interpreted" ;;
        -c) modes="compiled" ;;
        *) words="$words $a" ;;
    esac
done
if [ ! -x "$crow" ]; then
    echo "no compiler at $crow (build it, or set CROW)" >&2
    exit 1
fi
mkdir -p "$work"
for f in "$here"/data/*; do
    [ -f "$f" ] && ln -sf "$f" "$work/$(basename "$f")"
done

wanted() {
    [ -z "$words" ] && return 0
    for w in $words; do
        case "$1" in *"$w"*) return 0 ;; esac
    done
    return 1
}

run_one() { # mode program input -> output on stdout
    if [ "$1" = interpreted ]; then
        (cd "$work" && perl -e 'alarm shift; exec @ARGV' "$limit" "$crow" run -i "$2" < "$3" 2>&1)
    else
        (cd "$work" && perl -e 'alarm shift; exec @ARGV' "$limit" "$crow" run "$2" < "$3" 2>&1)
    fi
}

pass=0
fail=0
unrecorded=0
failed=""
for prog in "$here"/*.co; do
    slug=$(basename "$prog" .co)
    wanted "$slug" || continue
    input="$here/$slug.in"
    [ -f "$input" ] || input=/dev/null
    expect="$here/$slug.out"
    first=""
    ok=1
    for mode in $modes; do
        got=$(run_one "$mode" "$prog" "$input")
        if [ -f "$expect" ]; then
            want=$(cat "$expect")
        else
            want="$first"
        fi
        if [ -f "$expect" ] || [ -n "$first" ]; then
            if [ "$got" != "$want" ]; then
                ok=0
                line=$(diff <(printf '%s\n' "$want") <(printf '%s\n' "$got") | grep '^[<>]' | head -2 | tr '\n' ' ')
                echo "  FAIL  $slug ($mode): $line"
            fi
        fi
        [ -z "$first" ] && first="$got"
    done
    if [ $ok = 1 ]; then
        pass=$((pass + 1))
        if [ -f "$expect" ]; then
            echo "  ok    $slug"
        else
            unrecorded=$((unrecorded + 1))
            echo "  ok    $slug (no recorded output: the modes agree)"
        fi
    else
        fail=$((fail + 1))
        failed="$failed $slug"
    fi
done
echo
echo "$pass passed, $fail failed ($modes); $unrecorded without a recorded output"
[ $fail -eq 0 ] || { echo "failed:$failed"; exit 1; }
