#!/usr/bin/env bash
# Everything CI would run: formatting, lints and tests.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
