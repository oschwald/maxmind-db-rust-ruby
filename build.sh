#!/bin/bash
set -e

echo "Building maxmind-db-rust extension..."

# Build the Rust extension
cd ext/maxmind_db_rust
cargo build --release
cd ../..

# Copy to lib directory
mkdir -p lib/maxmind/db
cp ext/maxmind_db_rust/target/release/libmaxmind_db_rust.so lib/maxmind/db/maxmind_db_rust.so

echo "✓ Build complete! Extension is at lib/maxmind/db/maxmind_db_rust.so"
