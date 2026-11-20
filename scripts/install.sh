#!/bin/bash
set -euo pipefail

work_dir=""
install_stage=""
lock_dir=""
fail() { printf 'hm: %s\n' "$*" >&2; exit 1; }
cleanup() {
  [[ -z "$work_dir" ]] || rm -rf -- "$work_dir"
  [[ -z "$install_stage" ]] || rm -rf -- "$install_stage"
  [[ -z "$lock_dir" ]] || rmdir -- "$lock_dir"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

download() {
  curl --proto '=https' --proto-redir '=https' --tlsv1.2 \
    --fail --silent --show-error --location --retry 2 \
    --connect-timeout 15 --max-time 300 --output "$2" "$1"
}

configure_path() {
  local shell_name="${SHELL:-}" config="" candidate="" path_line='export PATH="$HOME/.local/bin:$PATH"'
  case ":$PATH:" in *":$HOME/.local/bin:"*) return ;; esac
  case "${shell_name##*/}" in
    zsh) config="${ZDOTDIR:-$HOME}/.zshrc" ;;
    bash)
      config="$HOME/.bash_profile"
      for candidate in "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile"; do
        if [[ -f "$candidate" ]]; then config="$candidate"; break; fi
      done
      ;;
  esac
  if [[ -n "$config" ]]; then
    if [[ -f "$config" ]] && grep -Fqx "$path_line" "$config"; then
      printf 'Open a new terminal to use hm.\n'; return
    fi
    if (printf '\n%s\n' "$path_line" >> "$config"); then
      printf 'Added ~/.local/bin to PATH in %s. Open a new terminal to use hm.\n' "$config"; return
    fi
  fi
  printf 'Add ~/.local/bin to PATH. You can also run %s/.local/bin/hm directly.\n' "$HOME"
}

check_root() {
  local root="$1"
  if [[ -e "$root" || -L "$root" ]]; then
    [[ -d "$root" && ! -L "$root" && -f "$root/.installer-managed" && ! -L "$root/.installer-managed" ]] \
      || fail "Refusing to use an unmanaged directory: $root"
  fi
}

acquire_lock() {
  mkdir "$1/.install-lock" 2>/dev/null || fail "Another installation may be running. If interrupted, remove $1/.install-lock and retry."
  lock_dir="$1/.install-lock"
}

authorization_directory() {
  printf '/usr/local/libexec/hm/%s\n' "$UID"
}

authorization_for() {
  local root="$1" source="$(authorization_directory)/source"
  [[ -d "$root" && -f "$source" && ! -L "$source" ]] \
    && [[ "$(cat "$source")" == "$(cd "$root" && pwd -P)" ]]
}

refresh_authorization() {
  local root="$1" version_dir="$2" helper="$(authorization_directory)/hm"
  if authorization_for "$root"; then
    [[ -f "$helper" && ! -L "$helper" && -x "$helper" ]] || fail 'Authorization is damaged. Run hm setup and retry.'
    if [[ "$("$helper" --version)" != "$("$version_dir/hm" --version)" ]]; then
      "$version_dir/hm" setup || fail 'Authorization update failed. The previous release remains selected.'
    fi
  fi
}

remove_authorization() {
  if authorization_for "$1"; then
    "$1/current/hm" setup --remove || fail 'Authorization removal failed. The installation was preserved.'
  fi
}

uninstall() {
  local root="$HOME/.local/share/hm" bin="$HOME/.local/bin/hm" target
  check_root "$root"
  if [[ -d "$root" ]]; then acquire_lock "$root"; fi
  remove_authorization "$root"
  if [[ -L "$bin" ]]; then
    target=$(readlink "$bin")
    if [[ "$target" == "$root/current/hm" ]]; then
      rm -- "$bin"
    else printf 'Preserved unrelated command: %s\n' "$bin"; fi
  elif [[ -e "$bin" ]]; then printf 'Preserved unrelated command: %s\n' "$bin"; fi
  if [[ -n "$lock_dir" ]]; then rm -rf -- "$root"; lock_dir=""; fi
  printf 'hm installer files removed. Hosts, backups and shell configuration were preserved.\n'
}

main() {
  [[ $# -le 1 ]] || fail 'Expected at most one option. Use --help for usage.'
  case "${1:-}" in
    --help|-h) printf 'Usage: install.sh [--uninstall | --help]\n\nInstall or update hm to the latest stable release.\n--uninstall  Remove installer-managed files\n'; return ;;
    ''|--uninstall) ;;
    *) fail "Unknown option: $1. Use --help for usage." ;;
  esac
  [[ "$(uname -s)" == Darwin && "$(uname -m)" == arm64 ]] || fail 'This installer supports Apple Silicon macOS only. Run from a native ARM terminal.'
  [[ "${HOME:-}" == /* && "$HOME" != / ]] || fail 'HOME must be an absolute user directory.'
  [[ "$EUID" -ne 0 ]] || fail 'Run this installer as your user, without sudo.'
  if [[ "${1:-}" == --uninstall ]]; then uninstall; return; fi
  local command
  for command in curl tar plutil shasum file; do command -v "$command" >/dev/null 2>&1 || fail "Required command not found: $command"; done
  local root="$HOME/.local/share/hm" bin="$HOME/.local/bin/hm" target
  check_root "$root"
  if [[ -e "$bin" || -L "$bin" ]]; then
    [[ -L "$bin" ]] || fail "Another hm command already exists at $bin. Move it before installing."
    target=$(readlink "$bin")
    [[ "$target" == "$root/current/hm" ]] || fail "Another hm command already exists at $bin."
  fi
  if [[ -e "$root/current" || -L "$root/current" ]]; then
    [[ -L "$root/current" ]] || fail "Refusing to replace $root/current: expected an installer symlink."
  fi
  work_dir=$(mktemp -d "${TMPDIR:-/tmp}/hm.XXXXXX")
  download 'https://api.github.com/repos/wereallcode/hm/releases/latest' "$work_dir/release.json" \
    || fail 'Could not fetch the latest release. A published GitHub Release is required.'
  local tag version_dir asset checksum actual listing
  tag=$(plutil -extract tag_name raw -o - "$work_dir/release.json") || fail 'GitHub did not return a release tag.'
  [[ "$tag" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || fail 'GitHub did not return a valid stable release.'
  [[ "$(plutil -extract draft raw -o - "$work_dir/release.json")" == false && "$(plutil -extract prerelease raw -o - "$work_dir/release.json")" == false ]] || fail 'GitHub did not return a stable published release.'
  asset="hm-$tag-aarch64-apple-darwin.tar.gz"
  version_dir="$root/$tag"
  mkdir -p "$root" "$HOME/.local/bin"
  acquire_lock "$root"
  touch "$root/.installer-managed"
  install_stage=$(mktemp -d "$root/.staging.XXXXXX")
  [[ ! -L "$version_dir" ]] || fail "Refusing to use a symlink as a release directory: $version_dir"
  if [[ -e "$version_dir" && ! -d "$version_dir" ]]; then fail "Refusing to replace an unrelated file: $version_dir"; fi
  if [[ ! -d "$version_dir" ]]; then
    printf 'Downloading hm %s...\n' "$tag"
    download "https://github.com/wereallcode/hm/releases/download/$tag/$asset" "$work_dir/$asset"
    download "https://github.com/wereallcode/hm/releases/download/$tag/SHA256SUMS" "$work_dir/SHA256SUMS"
    checksum=$(awk -v name="$asset" '$2 == name {print $1}' "$work_dir/SHA256SUMS")
    [[ "$checksum" =~ ^[a-f0-9]{64}$ ]] || fail 'Release checksum is missing or invalid.'
    actual=$(shasum -a 256 "$work_dir/$asset"); actual=${actual%% *}
    [[ "$actual" == "$checksum" ]] || fail 'Release checksum verification failed.'
    listing=$(tar -tzf "$work_dir/$asset" | LC_ALL=C sort)
    [[ "$listing" == $'LICENSE\nhm' ]] || fail 'Unexpected release archive contents.'
    mkdir "$install_stage/package"
    tar -xzf "$work_dir/$asset" -C "$install_stage/package"
    [[ -f "$install_stage/package/hm" && ! -L "$install_stage/package/hm" && -x "$install_stage/package/hm" ]] || fail 'Release binary is missing or unsafe.'
    [[ -f "$install_stage/package/LICENSE" && ! -L "$install_stage/package/LICENSE" ]] || fail 'Release license is missing or unsafe.'
    file -b "$install_stage/package/hm" | grep -q 'Mach-O 64-bit executable arm64' || fail 'Release binary is not an ARM macOS executable.'
    [[ "$("$install_stage/package/hm" --version)" == "${tag#v}" ]] || fail 'Release binary failed version verification.'
    mv "$install_stage/package" "$version_dir"
  fi
  [[ -f "$version_dir/hm" && ! -L "$version_dir/hm" && -x "$version_dir/hm" ]] || fail "Installed release is damaged: $version_dir"
  file -b "$version_dir/hm" | grep -q 'Mach-O 64-bit executable arm64' || fail "Installed release has an invalid binary: $version_dir"
  [[ "$("$version_dir/hm" --version)" == "${tag#v}" ]] || fail "Installed release is damaged: $version_dir"
  refresh_authorization "$root" "$version_dir"
  ln -s "$version_dir" "$install_stage/current"
  ln -s "$root/current/hm" "$install_stage/hm"
  /bin/mv -h -f "$install_stage/current" "$root/current"
  /bin/mv -h -f "$install_stage/hm" "$bin"
  printf 'Installed hm %s.\n' "$tag"
  configure_path
  if ! authorization_for "$root"; then printf 'Run hm setup once to enable password-free hosts changes.\n'; fi
  printf 'Run: hm --help\n'
}
if [[ "${BASH_SOURCE[0]:-$0}" == "$0" ]]; then main "$@"; fi
