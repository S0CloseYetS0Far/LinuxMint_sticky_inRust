#!/bin/sh
# Build the Rust program for meson: cargo-build.sh <source dir> <build dir> <output>
set -e
cargo build --release --locked --manifest-path "$1/Cargo.toml" --target-dir "$2/cargo-target"
cp "$2/cargo-target/release/sticky" "$3"
