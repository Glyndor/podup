#!/usr/bin/env bash
# What the installer does when the destination, or the name it stages under,
# is a link. These cases pin the behaviour as it is today; they decide
# nothing about whether it is the behaviour to keep.
#
# 1. A `podup` that is a symlink to a file elsewhere (a package manager's
#    layout, or an earlier hand-made link) is REPLACED by a regular file in
#    the install directory. The file the link pointed at is not written to:
#    it keeps its bytes and its inode, and is simply no longer what `podup`
#    resolves to. `podup update` differs, it resolves the link and replaces
#    the real file (internal/update/install/mod.rs); this test is the place
#    to change when the two are made to agree.
# 2. A dangling `podup` link is replaced the same way.
# 3. The staged name is predictable (`.podup.install-$$`). Whoever can write
#    to the install directory can plant a link under it before the install
#    runs. The install must not write through it: GNU `install` unlinks the
#    name before it creates the file, and this fixes that, so the file the
#    link points at keeps its bytes.
# 4. A PODUP_INSTALL_DIR that is itself a link installs into the directory it
#    points at.
#
# Each case also asserts that the install succeeded and left no staging, so a
# harness that failed early would not look like a link that was handled.
# Production helpers are loaded dynamically, so their consumers are not visible here.
# shellcheck disable=SC2034,SC2317,SC2329
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
INSTALL_SH="$HERE/install.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
pass=0
fail=0

check() {
	if [[ "$2" == "$3" ]]; then
		echo "ok    $1"
		pass=$((pass + 1))
	else
		echo "FAIL  $1 (expected '$2', got '$3')"
		fail=$((fail + 1))
	fi
}

HELPERS="$WORK/helpers.sh"
for name in install_binary installed_mode verify_no_downgrade compare_versions verify_version_self_test run_root; do
	awk "/^${name}\\(\\) {/,/^}$/" "$INSTALL_SH" >> "$HELPERS"
done
# shellcheck disable=SC1090
source "$HELPERS"

NEW_BODY='#!/bin/sh
echo "podup 1.2.3"
'
OLD_BODY='#!/bin/sh
echo "podup 1.0.0"
'

# install_into <install-dir>: run the production install_binary for v1.2.3.
install_into() {
	local dir="$1" scratch="$2" rc=0
	mkdir -p "$scratch"
	printf '%s' "$NEW_BODY" > "$scratch/payload"
	(
		set -euo pipefail
		INSTALL_DIR="$dir"
		TMP_DIR="$scratch"
		VERSION=v1.2.3
		REPO=Glyndor/podup
		ARTIFACT=podup-linux-x86_64
		BASE_URL=https://fixture.invalid
		log_info() { :; }
		log_ok() { :; }
		log_error() { printf '%s\n' "$*" >&2; }
		fail() { log_error "$*"; exit 1; }
		command() {
			if [[ "$*" == '-v gh' ]]; then return 1; fi
			builtin command "$@"
		}
		download() { cp "$scratch/payload" "$2"; }
		ed25519_verify() { return 0; }
		verify_checksum() { :; }
		install_binary
	) > "$scratch/output" 2>&1 || rc=$?
	return "$rc"
}

fingerprint() { printf '%s %s' "$(cksum < "$1")" "$(stat -c %i "$1")"; }
staging_count() { find "$1" -name '.podup.install-*' | wc -l | tr -d ' '; }

# 1. The target is a symlink to a real file.
d="$WORK/linked-target"
mkdir -p "$d/bin" "$d/real"
printf '%s' "$OLD_BODY" > "$d/real/podup"
chmod 755 "$d/real/podup"
ln -s ../real/podup "$d/bin/podup"
before="$(fingerprint "$d/real/podup")"
rc=0; install_into "$d/bin" "$d/tmp" || rc=$?
check "linked target: install succeeds" 0 "$rc"
check "linked target: podup is now a regular file" regular "$([[ -L "$d/bin/podup" ]] && echo link || echo regular)"
check "linked target: podup holds the new bytes" "$(printf '%s' "$NEW_BODY" | cksum)" "$(cksum < "$d/bin/podup")"
check "linked target: the file behind the link keeps its bytes and inode" "$before" "$(fingerprint "$d/real/podup")"
check "linked target: leaves no staging" 0 "$(staging_count "$d/bin")"

# 2. The target is a dangling symlink.
d="$WORK/dangling-target"
mkdir -p "$d/bin"
ln -s "$d/nowhere/podup" "$d/bin/podup"
rc=0; install_into "$d/bin" "$d/tmp" || rc=$?
check "dangling target: install succeeds" 0 "$rc"
check "dangling target: podup is now a regular file" regular "$([[ -L "$d/bin/podup" ]] && echo link || echo regular)"
check "dangling target: podup holds the new bytes" "$(printf '%s' "$NEW_BODY" | cksum)" "$(cksum < "$d/bin/podup")"
check "dangling target: nothing was created behind the link" 0 "$([[ -e "$d/nowhere" ]] && echo 1 || echo 0)"

# 3. A link planted under the staged name, one to an existing file and one
#    to a path that does not exist yet. $$ is the same in the subshell
#    install_into runs, so the planted name is the one the installer picks.
d="$WORK/planted-staging"
mkdir -p "$d/bin"
printf 'victim\n' > "$d/victim"
ln -s "$d/victim" "$d/bin/.podup.install-$$"
before="$(fingerprint "$d/victim")"
rc=0; install_into "$d/bin" "$d/tmp" || rc=$?
check "planted staging link: install succeeds" 0 "$rc"
check "planted staging link: the file it pointed at keeps its bytes and inode" "$before" "$(fingerprint "$d/victim")"
check "planted staging link: podup holds the new bytes" "$(printf '%s' "$NEW_BODY" | cksum)" "$(cksum < "$d/bin/podup")"
check "planted staging link: leaves no staging" 0 "$(staging_count "$d/bin")"

d="$WORK/planted-staging-dangling"
mkdir -p "$d/bin"
ln -s "$d/created-by-attacker" "$d/bin/.podup.install-$$"
rc=0; install_into "$d/bin" "$d/tmp" || rc=$?
check "dangling staging link: install succeeds" 0 "$rc"
check "dangling staging link: nothing was created where it pointed" 0 "$([[ -e "$d/created-by-attacker" ]] && echo 1 || echo 0)"
check "dangling staging link: podup holds the new bytes" "$(printf '%s' "$NEW_BODY" | cksum)" "$(cksum < "$d/bin/podup")"

# 4. The install directory is itself a link.
d="$WORK/linked-dir"
mkdir -p "$d/real"
ln -s real "$d/bin"
rc=0; install_into "$d/bin" "$d/tmp" || rc=$?
check "linked directory: install succeeds" 0 "$rc"
check "linked directory: the binary is in the directory the link points at" "$(printf '%s' "$NEW_BODY" | cksum)" "$(cksum < "$d/real/podup")"
check "linked directory: leaves no staging" 0 "$(staging_count "$d/real")"

printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[[ "$fail" -eq 0 ]]
