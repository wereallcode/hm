#!/bin/bash
set -euo pipefail
project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
scratch=$(mktemp -d "${TMPDIR:-/tmp}/hm-release-test.XXXXXX")
trap 'rm -rf -- "$scratch"' EXIT
source "$project_root/scripts/tests/helpers.sh"
fixture 2.3.4 "$scratch/release"
checkout="$scratch/build-2.3.4"
asset=hm-v2.3.4-aarch64-apple-darwin.tar.gz
(cd "$scratch/release" && shasum -a 256 -c SHA256SUMS)
mkdir "$scratch/unpacked"
tar -xzf "$scratch/release/$asset" -C "$scratch/unpacked"
[[ "$("$scratch/unpacked/hm" --version)" == 2.3.4 ]] || fail 'packaged version mismatch'
file "$scratch/unpacked/hm" | grep -q 'Mach-O 64-bit executable arm64' || fail 'wrong architecture'
[[ "$(tar -tzf "$scratch/release/$asset" | LC_ALL=C sort)" == $'LICENSE\nhm' ]] || fail 'archive contents'
# Upload patterns must find the packaged files before a release can succeed.
asset_index=0
for expected in "$asset" SHA256SUMS; do
  pattern=$(plutil -extract "plugins.4.1.assets.$asset_index" raw -o - "$project_root/.releaserc.json")
  matches=("$checkout"/$pattern)
  [[ ${#matches[@]} -eq 1 && -f "${matches[0]}" && "${matches[0]##*/}" == "$expected" ]] \
    || fail "Release upload pattern does not match $expected: $pattern"
  asset_index=$((asset_index+1))
done
# Changes must affect only hm, never dependency versions.
cp "$checkout/Cargo.lock" "$scratch/before.lock"
bash "$checkout/scripts/version.sh" 2.3.5
sed 's/version = "2.3.4"/version = "2.3.5"/' "$scratch/before.lock" > "$scratch/expected.lock"
cmp "$scratch/expected.lock" "$checkout/Cargo.lock" || fail 'dependency versions changed'
if bash "$checkout/scripts/package.sh" 2.3.5 > "$scratch/log" 2>&1; then fail 'stale binary was packaged'; fi
cp "$checkout/Cargo.toml" "$scratch/manifest"
for invalid in '' v1.2.3 01.2.3 1.2 1.2.3-beta '1.2.3;echo invalid'; do
  if bash "$checkout/scripts/version.sh" "$invalid" > "$scratch/log" 2>&1; then fail 'invalid version accepted'; fi
done
cmp "$checkout/Cargo.toml" "$scratch/manifest" || fail 'invalid version changed manifest'
printf 'Release packaging and version scenarios passed.\n'
