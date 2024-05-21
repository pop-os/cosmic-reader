#!/usr/bin/env bash

set -ex

#clear
cargo fmt
cargo build --release
export RUST_BACKTRACE=1
export RUST_LOG=cosmic_reader=info
target/release/cosmic-reader "$@" 2>&1 | tee target/log
cat target/log | grep "unknown op" | cut -d '"' -f2 | sort | uniq
