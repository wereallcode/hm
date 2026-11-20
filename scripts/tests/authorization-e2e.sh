#!/bin/bash
set -euo pipefail

fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
[[ "${GITHUB_ACTIONS:-}" == true && "${RUNNER_ENVIRONMENT:-}" == github-hosted \
  && "${RUNNER_OS:-}" == macOS && "$(uname -sm)" == 'Darwin arm64' ]] \
  || fail 'Authorization E2E tests require a disposable GitHub-hosted ARM macOS runner.'

root() { /usr/bin/sudo -n -- "$@"; }
root /usr/bin/true
command -v expect >/dev/null || fail 'expect is required.'
project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
account=hm_e2e
test_uid=55000
record="/Users/$account"
helper_directory="/usr/local/libexec/hm/$test_uid"
policy="/private/etc/sudoers.d/hm-$test_uid"
setup_lock="/usr/local/libexec/hm/.$test_uid.lock"
approval_policy=/private/etc/sudoers.d/zz-hm-e2e-approval
approval_stage=/private/etc/sudoers.d/.hm-e2e-approval
if root /usr/bin/dscl . -read "$record" >/dev/null 2>&1; then fail 'Test account already exists.'; fi
[[ -z "$(root /usr/bin/dscl . -search /Users UniqueID "$test_uid")" ]] || fail 'Test UID is already in use.'
for path in "$helper_directory" "$policy" "$setup_lock" "$approval_policy" "$approval_stage"; do
  [[ ! -e "$path" && ! -L "$path" ]] || fail "Test path already exists: $path"
done

scratch=$(mktemp -d /Users/Shared/hm-authorization-test.XXXXXX)
runner_uid=$(id -u)
runner_gid=$(id -g)
saved_hosts=false
had_backup=false
had_lock=false
created_account=false
[[ ! -e /private/etc/.hosts.hm.lock ]] || had_lock=true
cleanup() {
  local result=$?
  trap - EXIT
  set +e
  root /bin/rm -f -- "$approval_policy" "$approval_stage" "$policy" "$setup_lock" || result=1
  root /bin/rm -rf -- "$helper_directory" || result=1
  if [[ "$saved_hosts" == true ]]; then
    root /bin/cp -p "$scratch/hosts" /private/etc/hosts || result=1
    cmp -s "$scratch/hosts" /private/etc/hosts || result=1
    if [[ "$had_backup" == true ]]; then
      root /bin/cp -p "$scratch/hosts.bak" /private/etc/hosts.bak || result=1
      root /usr/bin/cmp -s "$scratch/hosts.bak" /private/etc/hosts.bak || result=1
    else
      root /bin/rm -f /private/etc/hosts.bak || result=1
    fi
    if [[ "$had_lock" == false ]]; then root /bin/rm -f /private/etc/.hosts.hm.lock || result=1; fi
  fi
  if [[ "$created_account" == true ]]; then root /usr/bin/dscl . -delete "$record" || result=1; fi
  root /usr/sbin/chown -R "$runner_uid:$runner_gid" "$scratch" || result=1
  /bin/rm -rf -- "$scratch" || result=1
  exit "$result"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
chmod 755 "$scratch"
root /bin/cp -p /private/etc/hosts "$scratch/hosts"
if [[ -e /private/etc/hosts.bak ]]; then
  root /bin/cp -p /private/etc/hosts.bak "$scratch/hosts.bak"
  had_backup=true
fi
saved_hosts=true

source "$project_root/scripts/tests/helpers.sh"
fixture 2.3.4 "$scratch/old"
fixture 2.3.5 "$scratch/new"
chmod 755 "$scratch/old" "$scratch/new"
chmod a+r "$scratch/old/"* "$scratch/new/"*
mkdir "$scratch/bin" "$scratch/home"
cp "$project_root/scripts/install.sh" "$scratch/install.sh"
cat > "$scratch/bin/curl" <<'EOF'
#!/bin/bash
set -euo pipefail
output=""
url=""
while [[ $# -gt 0 ]]; do
  case "$1" in --output) output="$2"; shift 2 ;; *) url="$1"; shift ;; esac
done
name=${url##*/}
[[ "$name" != latest ]] || name=release.json
exec /bin/cp "${HM_E2E_FIXTURE:?}/$name" "$output"
EOF
chmod 755 "$scratch/bin/curl" "$scratch/install.sh"
test_path="$scratch/bin:/usr/bin:/bin:/usr/sbin:/sbin"
run_as() { /usr/bin/sudo -H -u "$account" -- /usr/bin/env PATH="$test_path" SHELL=/bin/bash "$@"; }
test_password=$(uuidgen)
printf '::add-mask::%s\n' "$test_password"
root /usr/bin/dscl . -create "$record"
created_account=true
root /usr/bin/dscl . -create "$record" UniqueID "$test_uid"
root /usr/bin/dscl . -create "$record" GeneratedUID "$(uuidgen)"
root /usr/bin/dscl . -create "$record" RealName 'hm authorization tests'
root /usr/bin/dscl . -create "$record" PrimaryGroupID 20
root /usr/bin/dscl . -create "$record" UserShell /bin/bash
root /usr/bin/dscl . -create "$record" NFSHomeDirectory "$scratch/home"
root /usr/bin/dscl . -passwd "$record" "$test_password"
root /usr/sbin/chown "$test_uid:20" "$scratch/home"
install_root="$scratch/home/.local/share/hm"
old_client="$install_root/v2.3.4/hm"
new_client="$install_root/v2.3.5/hm"
client="$install_root/current/hm"

# This rule represents administrator approval and never grants unrelated commands.
allow_approval() {
  printf '#%s ALL=(root) PASSWD: NOSETENV: %s setup, %s setup --remove, %s setup, %s setup --remove\n' \
    "$test_uid" "$old_client" "$old_client" "$new_client" "$new_client" > "$scratch/approval"
  root /usr/bin/install -o root -g wheel -m 440 "$scratch/approval" "$approval_stage"
  root /usr/sbin/visudo -c -f "$approval_stage"
  root /bin/mv "$approval_stage" "$approval_policy"
}
drop_approval() {
  root /bin/rm -f "$approval_policy"
  if [[ -e "$policy" ]]; then run_as /usr/bin/sudo -K; fi
}
cat > "$scratch/approve.exp" <<'EOF'
set timeout 60
set prompts 0
spawn -noecho /usr/bin/sudo -H -u $env(HM_E2E_USER) -- /usr/bin/env PATH=$env(HM_E2E_PATH) SHELL=/bin/bash {*}$argv
expect {
  -re {(?i)password[^\r\n]*: *$} {
    incr prompts
    if {$prompts > 1} { puts stderr "Administrator approval rejected the test password."; exit 1 }
    if {$env(HM_E2E_MODE) eq "cancel"} { send -- "\003" } else { send -- "$env(HM_E2E_PASSWORD)\r" }
    exp_continue
  }
  eof {}
  timeout { catch {exec /bin/kill -TERM [exp_pid]}; puts stderr "Administrator approval timed out."; exit 1 }
}
set status [wait]
set code [lindex $status 3]
if {$prompts != 1} { puts stderr "Expected one administrator password prompt."; exit 1 }
if {$env(HM_E2E_MODE) eq "approve" && $code != 0} { puts stderr "Approved command failed."; exit 1 }
if {$env(HM_E2E_MODE) eq "cancel" && $code == 0} { puts stderr "Cancelled command succeeded."; exit 1 }
EOF
approval() {
  local mode="$1"
  shift
  allow_approval
  run_as /usr/bin/sudo -K
  HM_E2E_USER="$account" HM_E2E_PATH="$test_path" HM_E2E_PASSWORD="$test_password" HM_E2E_MODE="$mode" \
    /usr/bin/expect "$scratch/approve.exp" "$@"
  drop_approval
}
check_entries() {
  run_as "$client" list --json | python3 -c '
import json, sys
names = {sys.argv[1], "www." + sys.argv[1]}
entries = [e for e in json.load(sys.stdin) if names.intersection(e["hostnames"])]
if sys.argv[2] == "missing":
    assert not entries, "Removed entries remain"
else:
    assert {h for e in entries for h in e["hostnames"] if h in names} == names
    assert all(e["active"] == (sys.argv[2] == "active") for e in entries)
' "$1" "$2"
}

printf 'Checking real setup and password-free hosts mutations.\n'
run_as /usr/bin/env HM_E2E_FIXTURE="$scratch/old" /bin/bash "$scratch/install.sh"
approval approve "$client" setup
[[ "$(stat -f '%u:%g:%Lp' "$helper_directory/hm")" == 0:0:755 ]] || fail 'Unsafe helper ownership or mode.'
[[ "$(stat -f '%u:%g:%Lp' "$policy")" == 0:0:440 ]] || fail 'Unsafe policy ownership or mode.'
if run_as /usr/bin/sudo -n -- /usr/bin/id > "$scratch/denied.log" 2>&1; then fail 'Unrelated root command was allowed.'; fi
run_as "$client" add 127.0.0.1 hm-e2e.invalid
check_entries hm-e2e.invalid active
run_as "$client" toggle hm-e2e.invalid
check_entries hm-e2e.invalid inactive
run_as "$client" restore
check_entries hm-e2e.invalid active
run_as "$client" remove www.hm-e2e.invalid
check_entries hm-e2e.invalid missing
run_as "$client" add 127.0.0.1 hm-upgrade.invalid
cp /private/etc/hosts "$scratch/before-update"

printf 'Checking cancelled and approved installer updates.\n'
approval cancel /usr/bin/env HM_E2E_FIXTURE="$scratch/new" /bin/bash "$scratch/install.sh"
[[ "$(run_as "$client" --version)" == 2.3.4 && "$("$helper_directory/hm" --version)" == 2.3.4 ]] \
  || fail 'Cancelled update changed the selected client or helper.'
cmp -s "$scratch/before-update" /private/etc/hosts
approval approve /usr/bin/env HM_E2E_FIXTURE="$scratch/new" /bin/bash "$scratch/install.sh"
[[ "$(run_as "$client" --version)" == 2.3.5 && "$("$helper_directory/hm" --version)" == 2.3.5 ]] \
  || fail 'Approved update did not refresh both executables.'
cmp -s "$scratch/before-update" /private/etc/hosts
run_as "$client" toggle hm-upgrade.invalid
check_entries hm-upgrade.invalid inactive
if run_as "$old_client" add 127.0.0.1 outdated.invalid > "$scratch/outdated.log" 2>&1; then fail 'Outdated client was accepted.'; fi
grep -q 'Authorization needs an update' "$scratch/outdated.log"

printf 'Checking revocation and authorized uninstall.\n'
approval approve "$client" setup --remove
[[ ! -e "$policy" && ! -e "$helper_directory" ]] || fail 'Revocation left active authorization.'
cp /private/etc/hosts "$scratch/before-remove"
if run_as "$client" add 127.0.0.1 revoked.invalid > "$scratch/revoked.log" 2>&1; then fail 'Revoked authorization still allowed writes.'; fi
cmp -s "$scratch/before-remove" /private/etc/hosts
approval approve "$client" setup
approval approve /bin/bash "$scratch/install.sh" --uninstall
[[ ! -e "$install_root" && ! -e "$policy" && ! -e "$helper_directory" ]] || fail 'Uninstall left managed files.'
cmp -s "$scratch/before-remove" /private/etc/hosts
printf 'Authorization E2E scenarios passed.\n'
