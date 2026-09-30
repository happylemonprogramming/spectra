#!/usr/bin/env bash
# Everything CI would run: formatting, lints, tests, and the frontend build.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
(cd app && npm ci --silent && npm run build)
