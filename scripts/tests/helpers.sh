#!/bin/bash
# Source from test scripts after setting project_root and scratch.
fixture() {
  local version="$1" destination="$2"
  local checkout="$scratch/build-$version"
  mkdir -p "$checkout/scripts" "$destination"
  cp "$project_root/Cargo.toml" "$project_root/Cargo.lock" "$project_root/LICENSE" "$checkout/"
  cp -R "$project_root/src" "$checkout/src"
  cp -R "$project_root/tests" "$checkout/tests"
  cp "$project_root/scripts/"{version,build,package,prepare-release}.sh "$checkout/scripts/"
  (cd "$checkout" && CARGO_TARGET_DIR="$project_root/target" bash scripts/prepare-release.sh "$version") >&2
  cp "$checkout/release/"* "$destination/"
  printf '{"tag_name":"v%s","draft":false,"prerelease":false}\n' "$version" > "$destination/release.json"
}
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
