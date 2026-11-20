#!/bin/bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
[[ "$(uname -sm)" == 'Darwin arm64' ]] || { printf 'Build requires Apple Silicon macOS.\n' >&2; exit 1; }
export MACOSX_DEPLOYMENT_TARGET=11.0
cargo build --release --locked --target aarch64-apple-darwin
mkdir -p dist
cp "${CARGO_TARGET_DIR:-target}/aarch64-apple-darwin/release/hm" dist/hm
