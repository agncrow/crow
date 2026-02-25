#!/bin/bash
set -e
cd /Users/vijay/code/rust_projects/crow

echo "=== Building Crow (release) ===" > test_results.txt
cargo build --release 2>&1 | tail -5 >> test_results.txt
echo "BUILD_OK" >> test_results.txt

for f in bench_helloworld bench_fibonacci bench_nsieve_100k bench_spectral_norm_500 bench_nbody_500k bench_binarytrees_15 bench_merkletrees_15; do
  echo "--- $f ---" >> test_results.txt
  ./target/release/crow run "examples/${f}.crow" >> test_results.txt 2>&1
  echo "" >> test_results.txt
done
echo "=== ALL DONE ===" >> test_results.txt

