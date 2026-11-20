#!/bin/bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
bash scripts/version.sh "${1:-}"
bash scripts/build.sh
bash scripts/package.sh "$1"
