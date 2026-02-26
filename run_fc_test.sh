#!/bin/bash
cd /Users/vijay/code/rust_projects/crow
cargo build --release 2>&1 | tail -3
echo "=== INTERPRETER TEST ==="
./target/release/crow run -i examples/first_class_test.crow 2>&1
echo "=== EXIT: $? ==="

