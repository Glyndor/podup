#!/usr/bin/env bash
# A replayed latest tag must not replace a newer working installation.
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

# Extract production functions; only transport and cryptographic preconditions
# are stubbed in the flow cases, which exercise the real staging and rename.
HELPERS="$WORK/helpers.sh"
for name in compare_versions verify_no_downgrade resolve_release_tag installed_mode install_binary verify_version_self_test run_root; do
	awk "/^${name}\\(\\) {/,/^}$/" "$INSTALL_SH" >> "$HELPERS"
done
# shellcheck disable=SC1090
source "$HELPERS"

comparison() {
	local actual rc=0
	if declare -F compare_versions >/dev/null; then
		actual="$(compare_versions "$2" "$3")" || rc=$?
	else
		actual=missing
		rc=127
	fi
	check "$1" "$4" "$rc:$actual"
}

comparison 'minor components are numeric' 5.9.0 5.10.0 '0:-1'
comparison 'reverse minor components' 5.10.0 5.9.0 '0:1'
comparison 'equal versions' 5.15.0 5.15.0 '0:0'
comparison 'major takes precedence' 6.0.0 5.99.99 '0:1'
comparison 'lower major takes precedence' 4.99.99 5.0.0 '0:-1'
comparison 'higher patch' 5.15.10 5.15.9 '0:1'
comparison 'lower patch' 5.15.9 5.15.10 '0:-1'
comparison 'optional v on left' v5.9.0 5.10.0 '0:-1'
comparison 'optional v on right' 5.10.0 v5.9.0 '0:1'
comparison 'optional v on both' v5.15.0 v5.15.0 '0:0'
comparison 'leading zeros stay decimal' 5.09.0 5.9.0 '0:0'
comparison 'large components do not overflow' 5.9223372036854775808.0 5.9.0 '0:1'
comparison 'dev suffix on left is invalid' 5.15.0-dev 5.14.0 '1:'
comparison 'dev suffix on right is invalid' 5.14.0 v5.15.0-dev '1:'
comparison 'garbage on left is invalid' rubbish 5.14.0 '1:'
comparison 'garbage on right is invalid' 5.14.0 rubbish '1:'
comparison 'partial version is invalid' 5.15 5.14.0 '1:'
comparison 'extra component is invalid' 5.15.0.1 5.14.0 '1:'
comparison 'signed component is invalid' 5.+15.0 5.14.0 '1:'
comparison 'empty version is invalid' '' 5.14.0 '1:'

flow() {
	local name="$1" requested="$2" reported="$3" exit_code="$4" tag="$5" expected="$6"
	local dir="$WORK/$name" rc=0 before after
	mkdir -p "$dir/bin" "$dir/tmp"
	printf '#!/bin/sh\nprintf "%%s\\n" "%s"\nexit %s\n' "$reported" "$exit_code" > "$dir/bin/podup"
	chmod +x "$dir/bin/podup"
	before="$(cksum < "$dir/bin/podup") $(stat -c %i "$dir/bin/podup")"
	printf '#!/bin/sh\necho "podup %s"\n' "${tag#v}" > "$dir/payload"
	(
		set -euo pipefail
		INSTALL_DIR="$dir/bin"
		TMP_DIR="$dir/tmp"
		VERSION="$requested"
		REPO=Glyndor/podup
		ARTIFACT=podup-linux-x86_64
		BASE_URL=https://fixture.invalid
		log_info() { :; }
		log_ok() { :; }
		log_error() { printf '%s\n' "$*" >&2; }
		fail() { log_error "$*"; exit 1; }
		# Never invoke gh, even on a developer machine where it is installed.
		command() {
			if [[ "$*" == '-v gh' ]]; then return 1; fi
			builtin command "$@"
		}
		curl() { printf '{"tag_name":"%s"}\n' "$tag"; }
		download() {
			printf '%s\n' "$1" >> "$dir/downloads"
			cp "$dir/payload" "$2"
			if [[ "$name" == changed-during-download ]]; then
				printf '#!/bin/sh\necho "podup 5.16.0"\n' > "$INSTALL_DIR/podup"
			fi
		}
		ed25519_verify() { return 0; }
		verify_checksum() { :; }
		install_binary
	) > "$dir/output" 2>&1 || rc=$?
	check "$name exit status" "$expected" "$rc"
	if [[ "$expected" == 1 ]]; then
		after="$(cksum < "$dir/bin/podup") $(stat -c %i "$dir/bin/podup")"
		if [[ "$name" == changed-during-download ]]; then
			check "$name preserves newer target" 'podup 5.16.0' "$("$dir/bin/podup" --version)"
		else
			check "$name preserves bytes and inode" "$before" "$after"
			check "$name refuses before downloading" 0 "$(test -e "$dir/downloads" && echo 1 || echo 0)"
		fi
		check "$name reports downgrade and explicit override" 1 \
			"$(grep -qi 'downgrade' "$dir/output" && grep -q "PODUP_VERSION=$tag" "$dir/output" && echo 1 || echo 0)"
		check "$name leaves no staging" 0 "$(find "$dir/bin" -name '.podup.install-*' | wc -l | tr -d ' ')"
	else
		check "$name installs resolved payload" "$(cksum < "$dir/payload")" "$(cksum < "$dir/bin/podup")"
	fi
}

flow latest-replay latest 'podup 5.15.0' 0 v5.14.0 1
flow explicit-downgrade v5.14.0 'podup 5.15.0' 0 v5.14.0 0
flow same-version latest 'podup 5.15.0' 0 v5.15.0 0
flow upgrade latest 'podup 5.9.0' 0 v5.10.0 0
flow unreadable latest 'podup 5.15.0' 1 v5.14.0 0
flow malformed latest 'broken binary' 0 v5.14.0 0
flow dev-target latest 'podup 5.15.0-dev' 0 v5.14.0 0
flow prefixed-target latest 'podup version v5.15.0' 0 v5.14.0 1
flow changed-during-download latest 'podup 5.13.0' 0 v5.14.0 1

printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[[ "$fail" -eq 0 ]]
