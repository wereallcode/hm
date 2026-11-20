#!/bin/bash
set -euo pipefail
version="${1:-}"
[[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || { printf 'Invalid release version.\n' >&2; exit 1; }
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
manifest=$(mktemp "$root/Cargo.toml.XXXXXX")
lock=$(mktemp "$root/Cargo.lock.XXXXXX")
trap 'rm -f -- "$manifest" "$lock"' EXIT
awk -v version="$version" '
  /^\[/ { package = ($0 == "[package]") }
  package && /^version = / { $0 = "version = \"" version "\""; count++ }
  {print}
  END {if (count != 1) exit 1}
' Cargo.toml > "$manifest"
awk -v version="$version" '
  /^\[\[package\]\]/ { package = 0 }
  /^name = "hm"$/ { package = 1 }
  package && /^version = / { $0 = "version = \"" version "\""; count++ }
  {print}
  END {if (count != 1) exit 1}
' Cargo.lock > "$lock"
chmod 644 "$manifest" "$lock"
mv "$manifest" Cargo.toml
mv "$lock" Cargo.lock
cargo metadata --locked --offline --no-deps --format-version 1 >/dev/null
