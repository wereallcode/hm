#!/bin/bash
set -euo pipefail
project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
scratch=$(mktemp -d "${TMPDIR:-/tmp}/hm-installer-test.XXXXXX")
trap 'rm -rf -- "$scratch"' EXIT
source "$project_root/scripts/tests/helpers.sh"
fixture 1.2.3 "$scratch/releases/1.2.3"
fixture 1.2.4 "$scratch/releases/1.2.4"
mkdir "$scratch/tools" "$scratch/tmp"
# The installer and installed program only see macOS system tools, not Node or Rust.
for tool in awk basename bash cat chmod cp cut dirname file grep head ln mkdir mktemp mv plutil readlink rm rmdir sed shasum sort tar touch; do
  ln -s "$(command -v "$tool")" "$scratch/tools/$tool"
done
cat > "$scratch/tools/uname" <<'SH'
#!/bin/bash
case "$1" in -s) echo "${HM_TEST_PLATFORM:-Darwin}" ;; -m) echo "${HM_TEST_ARCH:-arm64}" ;; esac
SH
cat > "$scratch/tools/curl" <<'SH'
#!/bin/bash
set -euo pipefail
output=""; url=""
while [[ $# -gt 0 ]]; do
  case "$1" in --output) output="$2"; shift 2 ;; https://*) url="$1"; shift ;; *) shift ;; esac
done
[[ "$url" != *"${HM_TEST_DOWNLOAD_FAILURE:-this-never-matches}"* ]] || exit 22
case "$url" in
  https://api.github.com/repos/wereallcode/hm/releases/latest) source="$HM_TEST_RELEASE_DIR/release.json" ;;
  https://github.com/wereallcode/hm/releases/download/*) source="$HM_TEST_RELEASE_DIR/${url##*/}" ;;
  *) exit 22 ;;
esac
cp "$source" "$output"
SH
chmod +x "$scratch/tools/uname" "$scratch/tools/curl"
export HM_TEST_RELEASE_DIR="$scratch/releases/1.2.3"
count=0
new_home() {
  home="$scratch/home-$count"; count=$((count+1)); mkdir -p "$home"
  unset HM_TEST_DOWNLOAD_FAILURE HM_TEST_PLATFORM HM_TEST_ARCH
  export HM_TEST_RELEASE_DIR="$scratch/releases/1.2.3"
}
install() {
  HOME="$home" ZDOTDIR="$home" SHELL=/bin/zsh PATH="$scratch/tools" TMPDIR="$scratch/tmp" \
    /bin/bash "$project_root/scripts/install.sh" "$@" > "$scratch/log" 2>&1
}
expect_failure() { if install "$@"; then cat "$scratch/log"; fail 'installer unexpectedly succeeded'; fi; }
assert_version() {
  [[ "$("$home/.local/bin/hm" --version)" == "$1" ]] || fail 'installed version mismatch'
}

new_home
install || { cat "$scratch/log"; fail 'clean install'; }
assert_version 1.2.3
[[ "$(readlink "$home/.local/bin/hm")" == "$home/.local/share/hm/current/hm" ]] || fail 'command link'
[[ ! -e "$home/.local/share/hm/.install-lock" ]] || fail 'lock cleanup'
[[ "$(grep -c 'export PATH=' "$home/.zshrc")" == 1 ]] || fail 'PATH setup'
[[ ! -e "$scratch/tools/node" && ! -e "$scratch/tools/cargo" && ! -e "$scratch/tools/rustc" ]] || fail 'tool isolation'
printf '127.0.0.1 localhost\n' > "$home/hosts"
printf 'saved backup\n' > "$home/hosts.bak"
PATH="$scratch/tools" "$home/.local/bin/hm" --help | grep -q 'hm add' || fail 'installed CLI'
install || fail 'idempotent reinstall'
[[ "$(grep -c 'export PATH=' "$home/.zshrc")" == 1 ]] || fail 'duplicate PATH'

export HM_TEST_RELEASE_DIR="$scratch/releases/1.2.4"
export HM_TEST_DOWNLOAD_FAILURE='tar.gz'
expect_failure
assert_version 1.2.3
unset HM_TEST_DOWNLOAD_FAILURE
cp "$scratch/releases/1.2.4/SHA256SUMS" "$scratch/checksums"
printf '%064d  hm-v1.2.4-aarch64-apple-darwin.tar.gz\n' 0 > "$scratch/releases/1.2.4/SHA256SUMS"
expect_failure
assert_version 1.2.3
cp "$scratch/checksums" "$scratch/releases/1.2.4/SHA256SUMS"
install || { cat "$scratch/log"; fail 'upgrade'; }
assert_version 1.2.4
cp "$home/.zshrc" "$scratch/zshrc"
install --uninstall || fail 'uninstall'
[[ ! -e "$home/.local/share/hm" && ! -L "$home/.local/bin/hm" ]] || fail 'uninstall cleanup'
[[ -f "$home/hosts" && -f "$home/hosts.bak" ]] || fail 'uninstall touched hosts'
cmp "$home/.zshrc" "$scratch/zshrc" || fail 'uninstall touched shell'

for mode in platform arch; do
  new_home
  if [[ "$mode" == platform ]]; then export HM_TEST_PLATFORM=Linux; else export HM_TEST_ARCH=x86_64; fi
  expect_failure
  [[ ! -e "$home/.local" ]] || fail 'unsupported platform created files'
done
for arg in --unknown 'two arguments'; do new_home; expect_failure "$arg"; done
new_home; install --help || fail 'help'; [[ ! -e "$home/.local" ]] || fail 'help installed files'
HOME="$home" PATH="$scratch/tools" /bin/bash -s -- --help < "$project_root/scripts/install.sh" > "$scratch/log" 2>&1 \
  || { cat "$scratch/log"; fail 'piped installer help'; }

# Exercise authorization lifecycle hooks with isolated system paths and a CLI stub.
(
  source "$project_root/scripts/install.sh"
  authorization_directory() { printf '%s\n' "$scratch/authorization"; }
  auth_root="$scratch/authorized-install"
  auth_version="$scratch/authorized-version"
  mkdir -p "$auth_root" "$auth_version" "$scratch/authorization"
  export HM_TEST_AUTH_LOG="$scratch/authorization.log"
  printf '#!/bin/bash\nprintf "1.2.3\\n"\n' > "$scratch/authorization/hm"
  cat > "$auth_version/hm" <<'SH'
#!/bin/bash
if [[ "$1" == --version ]]; then printf '1.2.4\n'; exit; fi
printf '%s\n' "$*" >> "$HM_TEST_AUTH_LOG"
exit "${HM_TEST_AUTH_STATUS:-0}"
SH
  chmod +x "$scratch/authorization/hm" "$auth_version/hm"
  ln -s "$auth_version" "$auth_root/current"
  printf '%s' "$scratch/different-install" > "$scratch/authorization/source"
  refresh_authorization "$auth_root" "$auth_version"
  remove_authorization "$auth_root"
  [[ ! -e "$HM_TEST_AUTH_LOG" ]] || fail 'touched another installation authorization'
  (cd "$auth_root" && pwd -P | tr -d '\n') > "$scratch/authorization/source"
  refresh_authorization "$auth_root" "$auth_version"
  [[ "$(cat "$HM_TEST_AUTH_LOG")" == setup ]] || fail 'authorization update missing'
  printf '#!/bin/bash\nprintf "1.2.4\\n"\n' > "$scratch/authorization/hm"
  refresh_authorization "$auth_root" "$auth_version"
  [[ "$(cat "$HM_TEST_AUTH_LOG")" == setup ]] || fail 'unchanged helper requested approval'
  remove_authorization "$auth_root"
  [[ "$(tail -1 "$HM_TEST_AUTH_LOG")" == 'setup --remove' ]] || fail 'authorization removal missing'
  export HM_TEST_AUTH_STATUS=1
  if (remove_authorization "$auth_root" > "$scratch/log" 2>&1); then fail 'authorization cancellation ignored'; fi
  [[ -L "$auth_root/current" && -f "$auth_version/hm" ]] || fail 'cancelled removal deleted installation'
  printf '#!/bin/bash\nprintf "1.2.3\\n"\n' > "$scratch/authorization/hm"
  if (refresh_authorization "$auth_root" "$auth_version" > "$scratch/log" 2>&1); then fail 'authorization update cancellation ignored'; fi
  [[ -L "$auth_root/current" ]] || fail 'cancelled authorization changed current release'
)

for mode in unmanaged root-link marker-link command-file command-link current-file lock version-link; do
  new_home; root="$home/.local/share/hm"; mkdir -p "$home/.local/share" "$home/.local/bin" "$home/outside"
  if [[ "$mode" == root-link ]]; then ln -s "$home/outside" "$root"; else mkdir "$root"; touch "$root/.installer-managed"; fi
  case "$mode" in
    unmanaged) rm "$root/.installer-managed" ;;
    marker-link) rm "$root/.installer-managed"; touch "$home/outside/marker"; ln -s "$home/outside/marker" "$root/.installer-managed" ;;
    command-file) echo unrelated > "$home/.local/bin/hm" ;;
    command-link) ln -s "$home/outside" "$home/.local/bin/hm" ;;
    current-file) echo unrelated > "$root/current" ;;
    lock) mkdir "$root/.install-lock" ;;
    version-link) ln -s "$home/outside" "$root/v1.2.3" ;;
  esac
  expect_failure
  [[ -d "$home/outside" ]] || fail 'outside directory removed'
  if [[ "$mode" == lock ]]; then [[ -d "$root/.install-lock" ]] || fail 'another lock removed'; fi
done

# Reject a correctly checksummed package containing the wrong binary version.
new_home
mkdir "$scratch/wrong" "$scratch/wrong-package"
cp "$scratch/releases/1.2.3/release.json" "$scratch/wrong/"
tar -xzf "$scratch/releases/1.2.4/hm-v1.2.4-aarch64-apple-darwin.tar.gz" -C "$scratch/wrong-package"
COPYFILE_DISABLE=1 tar -czf "$scratch/wrong/hm-v1.2.3-aarch64-apple-darwin.tar.gz" -C "$scratch/wrong-package" hm LICENSE
(cd "$scratch/wrong" && shasum -a 256 *.tar.gz > SHA256SUMS)
export HM_TEST_RELEASE_DIR="$scratch/wrong"
expect_failure
[[ ! -L "$home/.local/bin/hm" ]] || fail 'wrong version activated'

for mode in unmanaged root-link marker-link; do
  new_home; root="$home/.local/share/hm"; mkdir -p "$home/.local/share" "$home/outside"
  if [[ "$mode" == root-link ]]; then ln -s "$home/outside" "$root"; else mkdir "$root"; fi
  if [[ "$mode" == marker-link ]]; then touch "$home/outside/marker"; ln -s "$home/outside/marker" "$root/.installer-managed"; fi
  expect_failure --uninstall
  [[ -d "$home/outside" ]] || fail 'unsafe uninstall removed outside directory'
done
new_home; root="$home/.local/share/hm"; mkdir -p "$root"; touch "$root/.installer-managed"; mkdir "$root/.install-lock"
expect_failure --uninstall
[[ -d "$root/.install-lock" ]] || fail 'uninstall removed another lock'

printf 'Installer scenarios passed (%s isolated homes).\n' "$count"
