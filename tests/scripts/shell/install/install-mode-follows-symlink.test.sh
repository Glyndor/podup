#!/usr/bin/env bash
# The installer gives the new binary the mode of what `podup` resolves to. It read
# the mode of the symlink itself (777 on Linux), so reinstalling over a link to a
# 0755 file left a binary every local user could rewrite.
set -uo pipefail
export LC_ALL=C
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
pass=0
fail=0
check() {
	if [ "$2" = "$3" ]; then
		echo "ok    $1"
		pass=$((pass + 1))
	else
		echo "FAIL  $1 (expected $2, got $3)"
		fail=$((fail + 1))
	fi
}
# Take the real function out of install.sh instead of copying it here.
fn="$(awk '/^installed_mode\(\) \{/,/^}/' "$HERE/install.sh")"
check "installed_mode is found in install.sh" 1 "$([ -n "$fn" ] && echo 1 || echo 0)"
eval "$fn"

mode_of() { installed_mode "$1"; }

touch "$WORK/plain" && chmod 0755 "$WORK/plain"
check "a regular 0755 file" 755 "$(mode_of "$WORK/plain")"
touch "$WORK/readonly" && chmod 0555 "$WORK/readonly"
check "a 0555 file keeps no write bit" 555 "$(mode_of "$WORK/readonly")"
touch "$WORK/suid" && chmod 4755 "$WORK/suid"
check "setuid is cut off" 755 "$(mode_of "$WORK/suid")"
ln -s plain "$WORK/link"
check "a symlink to a 0755 file is 755, not the 777 of the link" 755 "$(mode_of "$WORK/link")"
ln -s readonly "$WORK/link-ro"
check "a symlink to a 0555 file is 555" 555 "$(mode_of "$WORK/link-ro")"
ln -s missing "$WORK/dangling"
check "a dangling link is the default" 755 "$(mode_of "$WORK/dangling")"
check "a missing path is the default" 755 "$(mode_of "$WORK/nothing")"
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
