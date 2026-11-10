#!/usr/bin/env bash
# A release whose SHA256SUMS does not describe the downloaded artifact must be
# refused by the installer's own checksum gate, and the installation that was
# there must survive untouched.
#
# install-no-downgrade.test.sh replaces verify_checksum with a no-op to reach
# the staging code, so until this file nothing ran the real function: a
# regression that made it always pass would have gone unnoticed. Here the
# signature check is the only stub (it needs a key and python3, and has its own
# tests); verify_checksum is the production function, loaded from install.sh.
#
# Each refusal case is paired with a control that installs, with the same
# harness, a SHA256SUMS that does describe the artifact. Without the control a
# harness that always failed would satisfy every refusal.
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
for name in verify_checksum install_binary installed_mode verify_no_downgrade compare_versions verify_version_self_test run_root; do
	awk "/^${name}\\(\\) {/,/^}$/" "$INSTALL_SH" >> "$HELPERS"
done
# shellcheck disable=SC1090
source "$HELPERS"

ARTIFACT_NAME=podup-linux-x86_64
GOOD_SHA="$(printf 'good' | sha256sum | cut -d' ' -f1)"
BAD_SHA="$(printf 'evil' | sha256sum | cut -d' ' -f1)"

# flow <case> <exit> <sums-text>
#   Installs v1.2.3 over an existing podup. The downloaded artifact is a script
#   that reports 1.2.3, so a gate that let it through would install it.
flow() {
	local name="$1" expected="$2" sums="$3"
	local dir="$WORK/$name" rc=0 before after
	mkdir -p "$dir/bin" "$dir/tmp"
	printf '#!/bin/sh\necho "podup 1.0.0"\n' > "$dir/bin/podup"
	chmod 755 "$dir/bin/podup"
	before="$(cksum < "$dir/bin/podup") $(stat -c %i "$dir/bin/podup")"
	# The artifact is runnable, so a gate that let it through would install it.
	printf '#!/bin/sh\necho "podup 1.2.3"\n' > "$dir/payload"
	(
		set -euo pipefail
		INSTALL_DIR="$dir/bin"
		TMP_DIR="$dir/tmp"
		VERSION=v1.2.3
		REPO=Glyndor/podup
		ARTIFACT="$ARTIFACT_NAME"
		BASE_URL=https://fixture.invalid
		SHA256_CMD=(sha256sum)
		log_info() { :; }
		log_ok() { :; }
		log_error() { printf '%s\n' "$*" >&2; }
		fail() { log_error "$*"; exit 1; }
		command() {
			if [[ "$*" == '-v gh' ]]; then return 1; fi
			builtin command "$@"
		}
		download() {
			case "$1" in
				*/SHA256SUMS) printf '%s\n' "$sums" > "$2" ;;
				*) cp "$dir/payload" "$2" ;;
			esac
		}
		ed25519_verify() { return 0; }
		install_binary
	) > "$dir/output" 2>&1 || rc=$?
	check "$name exit status" "$expected" "$rc"
	if [[ "$expected" == 1 ]]; then
		after="$(cksum < "$dir/bin/podup") $(stat -c %i "$dir/bin/podup")"
		check "$name refuses at the checksum gate" 1 \
			"$(grep -q "Checksum verification failed for $ARTIFACT_NAME" "$dir/output" && echo 1 || echo 0)"
		check "$name preserves bytes and inode" "$before" "$after"
		check "$name leaves no staging" 0 "$(find "$dir/bin" -name '.podup.install-*' | wc -l | tr -d ' ')"
	else
		check "$name installs the artifact" "$(cksum < "$dir/payload")" "$(cksum < "$dir/bin/podup")"
	fi
}

# The control's digest has to describe the payload flow() writes.
PAYLOAD_SHA="$(printf '#!/bin/sh\necho "podup 1.2.3"\n' | sha256sum | cut -d' ' -f1)"

flow control-matching-digest 0 "$PAYLOAD_SHA  $ARTIFACT_NAME"
flow digest-of-other-bytes 1 "$BAD_SHA  $ARTIFACT_NAME"
flow digest-malformed 1 "not-a-digest  $ARTIFACT_NAME"
flow no-entry-for-artifact 1 "$PAYLOAD_SHA  podup-darwin-arm64"
flow empty-manifest 1 ""
flow second-entry-wrong 1 "$PAYLOAD_SHA  $ARTIFACT_NAME
$BAD_SHA  $ARTIFACT_NAME"

# The function on its own: the exit status is what install_binary relies on.
unit() {
	local name="$1" expected="$2" sums="$3" body="$4" rc=0
	local dir="$WORK/unit-$name"
	mkdir -p "$dir"
	printf '%s\n' "$sums" > "$dir/SHA256SUMS"
	printf '%s' "$body" > "$dir/$ARTIFACT_NAME"
	(
		set -euo pipefail
		TMP_DIR="$dir"
		SHA256_CMD=(sha256sum)
		log_info() { :; }
		log_ok() { :; }
		fail() { printf '%s\n' "$*" >&2; exit 1; }
		verify_checksum "$ARTIFACT_NAME"
	) > "$dir/output" 2>&1 || rc=$?
	check "verify_checksum $name" "$expected" "$rc"
}

unit matching-digest 0 "$GOOD_SHA  $ARTIFACT_NAME" good
unit digest-of-other-bytes 1 "$BAD_SHA  $ARTIFACT_NAME" good
unit truncated-artifact 1 "$GOOD_SHA  $ARTIFACT_NAME" goo

printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[[ "$fail" -eq 0 ]]
