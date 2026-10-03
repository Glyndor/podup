#!/usr/bin/env bash
#
# Resolve a Fedora Cloud Base-Generic-<version>.x86_64.qcow2 image, fetch the
# CHECKSUM in the same directory, download the image, and verify its
# SHA-256 against the checksum. Used by the Podman lane in podman-lane.yml.
#
# dl.fedoraproject.org answers 404 about half the time, in streaks longer than
# the 25 s that the step's old `--retry 5 --retry-delay 5 --retry-all-errors`
# covered. Five lane runs failed that way between 2026-10-01 and 2026-10-02;
# the rerun always passed, which is what made the failure look environmental
# until someone counted. download.fedoraproject.org is Fedora's mirror
# redirector and answered 200 through a mirror every time, so each fetch
# attempt tries the primary then the fallback before sleeping and going
# around again.
#
# dl is Fedora's authoritative server; download.fedoraproject.org hands the
# request to a public mirror, and that mirror is not Fedora. The CHECKSUM
# file is fetched from dl only -- a mirror that serves a tampered image
# could serve a matching CHECKSUM, and the PGP signature on the file is not
# checked here, so the value the verifier compares the SHA-256 against has
# to come from Fedora's own server. The image itself may still come from a
# mirror; the CHECKSUM catches a tampered or truncated download regardless
# of which host the bytes came from.
#
# Args:
#   <podman-major>   5 (Fedora 44) or 6 (rawhide).
#   <output-path>    Path to write the image to. Must be writable; existing
#                    files are resumed (-C -) rather than truncated.
#
# Stdout: the resolved image filename, one line. The step captures it into
# $GITHUB_ENV as VM_IMAGE=<name>.
#
# Environment overrides (defaults are the production ones):
#   FEDORA_PRIMARY    default https://dl.fedoraproject.org
#   FEDORA_FALLBACK   default https://download.fedoraproject.org
#   FETCH_SLEEP       seconds between retries (default 15; 0 in tests)
#   FETCH_DEADLINE    overall wall-clock cap in seconds (default 1200)
#
# Exit codes: 0 success; otherwise 1 with an `::error::` annotation naming
# the failure.
#
# Requires: bash, curl, sha256sum, awk, grep, sed, sort.
set -euo pipefail
export LC_ALL=C

if [ "$#" -ne 2 ]; then
	echo "usage: $0 <podman-major> <output-path>" >&2
	exit 2
fi

MAJOR="$1"
OUT="$2"

: "${FEDORA_PRIMARY:=https://dl.fedoraproject.org}"
: "${FEDORA_FALLBACK:=https://download.fedoraproject.org}"
: "${FETCH_SLEEP:=15}"
# Overall wall-clock cap in seconds for the whole fetch. The lane job has
# `timeout-minutes: 95` and the VM step after this one may use up to 4200 s
# of it, so 1200 s here keeps the sum inside the job. Without this cap, 8
# tries per phase at the per-call limits below could run for hours. The
# check runs before each attempt, so one image call already in flight can
# overrun it by up to its own 600 s limit.
: "${FETCH_DEADLINE:=1200}"
# 8 tries spaced 15 s apart is roughly two minutes of backoff plus the curl
# time per try, which covers the longest streak dl.fedoraproject.org showed
# on 2026-10-01/02. The same TRIES cover the checksum and image downloads.
TRIES=8
# Metadata requests (index, CHECKSUM) cap a single curl call at 60s and
# refuse to wait more than 10s for the TCP handshake; the directory
# listing and the ~2 KB CHECKSUM are small, so a hung connection is
# almost always a dead server rather than a slow one, and 60s is more
# than enough to clear a real one. The image keeps the 600s budget
# because the file is ~500 MB and a slow mirror needs the room.
CURL_META_CONNECT_TIMEOUT=10
CURL_META_MAX_TIME=60
CURL_IMAGE_MAX_TIME=600
# Anything below 50 KB/s for 30 s is a hung mirror, not a slow one. curl
# aborts; the retry loop tries the next host.
CURL_SPEED_LIMIT=50000
CURL_SPEED_TIME=30
# The deadline can be hit before TRIES runs out. The flag below
# records that the loop bailed because of the deadline rather than
# exhaustion, so the existing failure path can append a note that
# names the deadline to the error it already prints.
DEADLINE_HIT=0

# The directory on each host that holds the cloud images for the release.
# The IMAGE_PATTERN is the same one the workflow grep used; Podman 5 boots
# against Fedora 44, Podman 6 against rawhide.
case "$MAJOR" in
	5)
		IMAGES_PATH="/pub/fedora/linux/releases/44/Cloud/x86_64/images/"
		IMAGE_PATTERN='Fedora-Cloud-Base-Generic-44-[0-9.]+\.x86_64\.qcow2'
		;;
	6)
		IMAGES_PATH="/pub/fedora/linux/development/rawhide/Cloud/x86_64/images/"
		IMAGE_PATTERN='Fedora-Cloud-Base-Generic-Rawhide-[0-9]+\.n\.[0-9]+\.x86_64\.qcow2'
		;;
	*)
		echo "::error::unsupported Podman major '$MAJOR' (expected 5 or 6)" >&2
		exit 1
		;;
esac

# Every CHECKSUM file in the directory ends in `-CHECKSUM`. The index lists
# exactly one, and grep picks the highest-versioned one as a tiebreak against
# a future directory layout. The character class includes `_` because
# rawhide's CHECKSUM embeds `images_Rawhide_x86_64` in the filename; without
# it the regex would stop at `x86` and the trailing `-CHECKSUM` would be
# matched against `-64-CHECKSUM`'s last `-`, picking the wrong substring.
CHECKSUM_PATTERN='[A-Za-z0-9._-]+-CHECKSUM'

# --- curl helpers -------------------------------------------------------------
#
# Every curl call shares the same bandwidth guard. `--max-time` caps the
# whole operation; `--speed-limit/--speed-time` aborts a mirror that
# returns bytes too slowly. Both are needed: the first can run out the
# budget on a slow mirror, the second catches a mirror that simply stops
# streaming after the connection succeeds.
curl_text() { # <url> -> stdout, returns 0 on 2xx, 22 on 4xx/5xx, otherwise curl's code
	curl -fsSL \
		--connect-timeout "$CURL_META_CONNECT_TIMEOUT" \
		--max-time "$CURL_META_MAX_TIME" \
		--speed-limit "$CURL_SPEED_LIMIT" \
		--speed-time "$CURL_SPEED_TIME" \
		"$1" 2>/dev/null
}

# Same flags plus resume (-C -) so a partial download picks up across
# retries. The budget is then spent on the remaining bytes rather than
# re-fetching the whole ~500 MB from byte 0 every attempt. The image
# keeps the full 600s max-time because the file is ~500 MB and a slow
# mirror needs the room; the metadata calls above use the smaller
# 60s/10s budget because the bytes are tiny and a hung handshake is a
# dead server.
curl_file() { # <url> <out> -> writes body to <out>, returns curl's code
	curl -fSL -C - \
		--max-time "$CURL_IMAGE_MAX_TIME" \
		--speed-limit "$CURL_SPEED_LIMIT" \
		--speed-time "$CURL_SPEED_TIME" \
		-o "$2" "$1" 2>/dev/null
}

# Try the primary, then the fallback. The first success wins; if both
# fail, the caller (the retry loop below) sleeps and tries again. Used
# for the index and the image, where falling back to a mirror is acceptable
# because the CHECKSUM below independently proves the bytes were not tampered
# with.
fetch_text() { # <path> -> body on stdout
	if body=$(curl_text "${FEDORA_PRIMARY}$1"); then
		printf '%s' "$body"
		return 0
	fi
	if body=$(curl_text "${FEDORA_FALLBACK}$1"); then
		printf '%s' "$body"
		return 0
	fi
	return 1
}

fetch_file() { # <path> <out>
	if curl_file "${FEDORA_PRIMARY}$1" "$2"; then
		return 0
	fi
	curl_file "${FEDORA_FALLBACK}$1" "$2"
}

# Fetch from the primary only, with no fallback. Used for the CHECKSUM
# file: the SHA-256 the verifier compares the image against has to come
# from Fedora's own server, not a mirror redirector that picked a public
# mirror, and the PGP signature is not checked here. Redirects are
# rejected with curl exit 47 so a redirector cannot substitute a mirror's
# CHECKSUM for Fedora's; the index and the image keep following
# redirects because the mirror redirector depends on that. The
# metadata-sized timeouts (10s connect, 60s total) match the index
# calls because the CHECKSUM is also a small file.
fetch_text_primary() { # <path> -> body on stdout
	curl -fsS --max-redirs 0 \
		--connect-timeout "$CURL_META_CONNECT_TIMEOUT" \
		--max-time "$CURL_META_MAX_TIME" \
		--speed-limit "$CURL_SPEED_LIMIT" \
		--speed-time "$CURL_SPEED_TIME" \
		"${FEDORA_PRIMARY}$1" 2>/dev/null
}

# `sleep` if there are more tries left AND the caller asked for a sleep
# (FETCH_SLEEP=0 in the unit test, where sleep would explode the run).
maybe_sleep() {
	if [ "$TRIES_LEFT" -le 0 ]; then
		return
	fi
	if [ "${FETCH_SLEEP:-0}" -gt 0 ] 2>/dev/null; then
		sleep "$FETCH_SLEEP"
	fi
}

# Return 0 when the overall wall-clock budget has run out. Each phase
# calls this at the top of its loop and breaks with DEADLINE_HIT=1 so the
# existing failure path can append the deadline note to the error.
deadline_reached() {
	[ "$SECONDS" -ge "$FETCH_DEADLINE" ]
}

# --- phase 1: resolve image and CHECKSUM filenames --------------------------------
#
# 8 attempts, each trying primary then fallback. After every failure sleep
# FETCH_SLEEP seconds. The empty-name check uses the canonical primary
# URL in the error because that is what the step's old message named.
TRIES_LEFT=$TRIES
checksum=""
img=""
while [ "$TRIES_LEFT" -gt 0 ]; do
	if deadline_reached; then
		DEADLINE_HIT=1
		break
	fi
	if index=$(fetch_text "$IMAGES_PATH"); then
		img=$(printf '%s' "$index" | grep -oE "$IMAGE_PATTERN" | LC_ALL=C sort -uV | tail -1 || true)
		checksum=$(printf '%s' "$index" | grep -oE "$CHECKSUM_PATTERN" | LC_ALL=C sort -uV | tail -1 || true)
		if [ -n "$img" ] && [ -n "$checksum" ]; then
			break
		fi
	fi
	TRIES_LEFT=$((TRIES_LEFT - 1))
	maybe_sleep
done

if [ -z "$img" ]; then
	msg="could not resolve a Fedora image name for Podman $MAJOR at ${FEDORA_PRIMARY}${IMAGES_PATH}"
	if [ "$DEADLINE_HIT" -eq 1 ]; then
		msg="$msg (fetch deadline of ${FETCH_DEADLINE}s reached)"
	fi
	echo "::error::$msg" >&2
	exit 1
fi
if [ -z "$checksum" ]; then
	# The phase 1 loop breaks only when both names resolve, but reaching
	# this branch with an image and no CHECKSUM means the loop exited
	# for some other reason and left checksum empty. Fail here rather
	# than letting phase 2 fetch the images directory itself as if it
	# were the CHECKSUM.
	msg="could not find a CHECKSUM link in ${FEDORA_PRIMARY}${IMAGES_PATH} for Podman $MAJOR"
	if [ "$DEADLINE_HIT" -eq 1 ]; then
		msg="$msg (fetch deadline of ${FETCH_DEADLINE}s reached)"
	fi
	echo "::error::$msg" >&2
	exit 1
fi

# --- phase 2: fetch the CHECKSUM ----------------------------------------------
#
# Same retry shape, but the CHECKSUM comes only from the primary (see
# fetch_text_primary above). If the primary answers 404 for every attempt,
# the loop runs out and the script fails with the error below -- the
# CHECKSUM is the value the image is verified against, and a missing one
# from Fedora's own server is not something a mirror can substitute for.
# The CHECKSUM file is small (~2 KB), so the timeout is the only budget
# pressure.
#
# The fetch result is captured into a local first; assigning it to
# checksum_body only when curl exited 0 keeps a partial body from a
# failed attempt (curl exit 18 or 28) from being mistaken for a real
# download after the retry budget runs out.
TRIES_LEFT=$TRIES
checksum_body=""
while [ "$TRIES_LEFT" -gt 0 ]; do
	if deadline_reached; then
		DEADLINE_HIT=1
		break
	fi
	if attempt=$(fetch_text_primary "${IMAGES_PATH}${checksum}"); then
		checksum_body="$attempt"
		unset attempt
		break
	fi
	unset attempt
	TRIES_LEFT=$((TRIES_LEFT - 1))
	maybe_sleep
done

if [ -z "$checksum_body" ]; then
	msg="could not fetch ${checksum} from ${FEDORA_PRIMARY} for Podman $MAJOR after $TRIES attempts"
	if [ "$DEADLINE_HIT" -eq 1 ]; then
		msg="$msg (fetch deadline of ${FETCH_DEADLINE}s reached)"
	fi
	echo "::error::$msg" >&2
	exit 1
fi

# Pull the SHA-256 line for the resolved image out of the CHECKSUM file.
# Skip the PGP signature armour (BEGIN PGP SIGNATURE through END PGP
# SIGNATURE) so the signature bytes are not mistaken for hashes; the
# MESSAGE block between the armour pairs is where the SHA-256 line lives,
# and the PGP signature on it is not checked here. The whole-line shape
# pins the digest to exactly 64 lowercase hex digits so a corrupted
# transport cannot turn the line into a record the verifier would
# accept. The parenthesised filename must match the resolved image
# exactly: matching the substring would let a CHECKSUM line that names a
# different file sneak in, and a CHECKSUM that records two digests for
# the same image is treated as conflicting rather than ambiguous.
awk_rc=0
expected=$(printf '%s' "$checksum_body" | awk -v img="$img" '
	BEGIN { count = 0; digest = ""; in_pgp = 0 }
	/-----BEGIN PGP SIGNATURE/ { in_pgp = 1; next }
	/-----END PGP SIGNATURE/ { in_pgp = 0; next }
	in_pgp { next }
	{
		# Whole line: "SHA256 (" + name + ") = " + digest, the digest
		# exactly 64 lowercase hex digits and the name exactly img. The
		# length check stands in for a {64} interval, which older mawk
		# builds do not support.
		if ($0 ~ /^SHA256 \([^)]+\) = [0-9a-f]+$/) {
			d = $NF
			if (length(d) != 64) next
			name = substr($0, 9, length($0) - 76)
			if (name != img) next
			if (count == 0) { digest = d; count = 1 }
			else if (d != digest) { count = 2 }
		}
	}
	END {
		if (count == 2) exit 3
		if (count == 1) { print digest; exit 0 }
		exit 0
	}
') || awk_rc=$?
if [ "$awk_rc" -eq 3 ]; then
	echo "::error::conflicting SHA-256 records for $img in ${checksum}" >&2
	exit 1
fi
if [ -z "$expected" ]; then
	echo "::error::could not find SHA-256 for $img in ${checksum}" >&2
	exit 1
fi

# --- phase 3: download the image ----------------------------------------------
#
# Same retry shape; resume (-C -) so a partial download picks up. If the
# whole budget is spent without a successful fetch, the loop ends with an
# empty output file and the verification step below fails loudly.
TRIES_LEFT=$TRIES
while [ "$TRIES_LEFT" -gt 0 ]; do
	if deadline_reached; then
		DEADLINE_HIT=1
		break
	fi
	if fetch_file "${IMAGES_PATH}${img}" "$OUT"; then
		break
	fi
	TRIES_LEFT=$((TRIES_LEFT - 1))
	maybe_sleep
done

if [ ! -s "$OUT" ]; then
	msg="could not download $img for Podman $MAJOR after $TRIES attempts"
	if [ "$DEADLINE_HIT" -eq 1 ]; then
		msg="$msg (fetch deadline of ${FETCH_DEADLINE}s reached)"
	fi
	echo "::error::$msg" >&2
	exit 1
fi

# --- phase 4: verify SHA-256 --------------------------------------------------
#
# The expected hash came from Fedora's server (the CHECKSUM was fetched
# from the primary only, see phase 2). The image bytes may have come from
# a mirror -- download.fedoraproject.org hands the request to one, and the
# image fetch in phase 3 falls back to it. If the two do not match, the
# download was tampered with or truncated; either way the file is unusable,
# so delete it and fail.
actual=$(sha256sum "$OUT" | awk '{print $1}')
if [ "$actual" != "$expected" ]; then
	rm -f "$OUT"
	echo "::error::SHA-256 mismatch for $img: expected $expected, got $actual" >&2
	exit 1
fi

# The step captures stdout into VM_IMAGE. The trailing newline is what
# bash's $() strips.
printf '%s\n' "$img"