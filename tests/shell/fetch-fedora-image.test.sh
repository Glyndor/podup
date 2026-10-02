#!/usr/bin/env bash
#
# Behaviour tests for `.github/scripts/fetch-fedora-image.sh`.
#
# The lane pulls ~500 MB off dl.fedoraproject.org, which 404s about half
# the time in streaks longer than `--retry 5 --retry-delay 5` covers.
# The script retries up to 8 times, falls back to the mirror redirector
# download.fedoraproject.org for the index and the image, fetches the
# CHECKSUM from the primary only, and verifies the SHA-256 of what came
# down against the value the CHECKSUM carries. None of that has a real
# network in the loop. The stub curl below serves them from a temp tree
# per host and answers 404 for the first N calls to a host (configurable
# per case), 200 from the temp tree thereafter.
#
# Cases:
#   - primary 404 for the index twice, then 200: resolves and downloads
#   - primary always 404: CHECKSUM phase fails (primary-only)
#   - both always 404: fails with the `could not resolve` error
#   - primary serves the index but 404s the CHECKSUM on every call,
#     fallback has the image and a CHECKSUM matching it: fails with the
#     primary-only CHECKSUM error, no image file left behind
#   - primary 404s the CHECKSUM twice, then serves it: succeeds
#   - checksum mismatch: fails, the error names both hashes, output gone
#   - the major selects the right path and name pattern (5 for 44, 6 for
#     rawhide)
#
# FETCH_SLEEP=0 is set so the resolve loop does not sleep between tries;
# the worst-case run here is a handful of milliseconds.
#
# Requires: bash, coreutils, sha256sum, the script under test, nothing else.
set -u

cd "$(dirname "$0")/../.." || exit 1
script="$PWD/.github/scripts/fetch-fedora-image.sh"

if [ ! -x "$script" ]; then
	echo "FAIL  $script is missing or not executable"
	exit 1
fi

pass=0
fail=0

check() { # <description> <expected> <actual>
	if [ "$2" = "$3" ]; then
		echo "ok    $1"
		pass=$((pass + 1))
	else
		echo "FAIL  $1"
		echo "        expected: $2"
		echo "        actual:   $3"
		fail=$((fail + 1))
	fi
}

# --- shared stub --------------------------------------------------------------
#
# Stub curl that:
#   1. parses the URL into host + path (query strings are stripped),
#   2. counts how many times each host has been called, persisting the
#      count under $STUB_COUNTERS_DIR/<host>,
#   3. appends the path to $STUB_COUNTERS_DIR/<host>.log so the test can
#      read which paths each host was called for (used to assert the
#      CHECKSUM never reached the fallback),
#   4. counts how many times each (host, path) pair has been called, under
#      $STUB_COUNTERS_DIR/<host>.<sanitized-path>, where the sanitizer
#      turns `/` and `.` into `_` so the result is a single filename,
#   5. decides the 404 limit for this call:
#      - if $STUB_LIMITS_DIR/<host>/<path> exists, its content is the
#        per-(host,path) limit; the per-(host,path) counter is compared
#        against it,
#      - otherwise STUB_404_<host> is the per-host limit; the per-host
#        counter is compared against it.
#   6. if the call count for the chosen scope is <= that limit, exits with
#      curl's HTTP error code (22) so the caller's `curl -f` is honoured,
#   7. otherwise copies $STUB_ROOT/<host>/<path> to stdout, or to `-o`'d
#      file when the script used -o. A URL ending in `/` is treated as a
#      directory listing (Apache serves index.html for that path), so the
#      stub mirrors it as well.
#
# The stub is POSIX sh because it runs as /bin/sh from the caller's PATH.
# The heredoc is quoted ('STUB') on purpose so the $1, $STUB_* etc. are
# written verbatim to the file -- the stub expands them at run time from
# its own environment.
write_stub() { # <path>
	cat > "$1" <<'STUB'
#!/bin/sh
set -eu
COUNTERS="$STUB_COUNTERS_DIR"
ROOT="$STUB_ROOT"
LIMITS="$STUB_LIMITS_DIR"
out=""
url=""
while [ $# -gt 0 ]; do
	case "$1" in
		--)
			shift
			url="${1:-}"
			break
			;;
		-o)
			out="$2"
			shift 2
			;;
		-*)
			shift
			;;
		*)
			url="$1"
			shift
			;;
	esac
done
if [ -z "$url" ]; then
	echo "stub curl: no URL in args" >&2
	exit 2
fi
host_path="${url#*://}"
host="${host_path%%/*}"
path="${host_path#*/}"
path="${path%%\?*}"
# Per-host counter (existing behaviour).
counter_file="$COUNTERS/$host"
if [ ! -f "$counter_file" ]; then
	echo 0 > "$counter_file"
fi
count=$(cat "$counter_file")
count=$((count + 1))
echo "$count" > "$counter_file"
# Per-host call log: every path this host has been asked for, one per
# line, in arrival order. Tests grep this to assert which paths went
# where (the CHECKSUM must never appear in the fallback's log).
printf '%s\n' "$path" >> "$COUNTERS/$host.log"
# Per-(host, path) counter. `/` and `.` collapse to `_` so the result
# fits a single filename; the original path is still on the per-host
# log above for human inspection.
sanitized=$(printf '%s' "$path" | tr '/.' '__')
path_counter="$COUNTERS/${host}.${sanitized}"
if [ ! -f "$path_counter" ]; then
	echo 0 > "$path_counter"
fi
path_count=$(cat "$path_counter")
path_count=$((path_count + 1))
echo "$path_count" > "$path_counter"
# Resolve the 404 limit. A per-(host, path) limit in $LIMITS overrides
# the per-host STUB_404_<host> for that path.
limit_file="$LIMITS/$host/$path"
if [ -n "$LIMITS" ] && [ -f "$limit_file" ]; then
	limit=$(cat "$limit_file")
	scope_count="$path_count"
else
	limit_var="STUB_404_$(printf "%s" "$host" | tr "." "_")"
	limit=$(eval "echo \${$limit_var:-0}")
	scope_count="$count"
fi
if [ "$scope_count" -le "$limit" ]; then
	if [ -n "$out" ]; then
		: > "$out"
	fi
	exit 22
fi
file="$ROOT/$host/$path"
# A URL ending in `/` is a directory listing. Apache serves index.html
# for that path; the stub mirrors that so the script's directory-fetch
# call lands on the planted file.
case "$path" in
	*/) file="$file"index.html ;;
esac
if [ ! -f "$file" ]; then
	if [ -n "$out" ]; then
		: > "$out"
	fi
	exit 22
fi
if [ -n "$out" ]; then
	cat "$file" > "$out"
else
	cat "$file"
fi
STUB
}

# --- case setup ---------------------------------------------------------------
#
# Build the fixture tree (stub curl + per-host content roots) and reset
# the per-host 404 counters and limits. Each case owns a fresh $WORK so
# cross-case state cannot leak. The caller sets env to inject per-host
# 404 limits (STUB_404_<host>) and per-path 404 limits (a file under
# $LIMITS/) before invoking the script.
setup() {
	WORK="$(mktemp -d)"
	STUB_DIR="$WORK/stub"
	ROOT="$WORK/roots"
	COUNTERS="$WORK/counters"
	LIMITS="$WORK/limits"
	mkdir -p "$STUB_DIR" "$ROOT" "$COUNTERS" "$LIMITS"

	write_stub "$STUB_DIR/curl"
	chmod +x "$STUB_DIR/curl"

	# Per-case 404 limits are exported before run_script so the stub sees
	# them. shellcheck cannot see across processes, so disable the unused
	# warning at the assignment site.
	export STUB_404_dl_fedoraproject_org="${STUB_404_dl_fedoraproject_org:-0}"
	export STUB_404_download_fedoraproject_org="${STUB_404_download_fedoraproject_org:-0}"
}

teardown() {
	[ -n "${WORK:-}" ] && rm -rf "$WORK"
	unset STUB_404_dl_fedoraproject_org
	unset STUB_404_download_fedoraproject_org
}

# --- fixtures -----------------------------------------------------------------
#
# A real Fedora 44 image name from the live index on 2026-10-02, with the
# corresponding CHECKSUM filename the index carries next to it.
IMG_44=Fedora-Cloud-Base-Generic-44-1.7.x86_64.qcow2
CS_44=Fedora-Cloud-44-1.7-x86_64-CHECKSUM

# Rawhide image name and the actual CHECKSUM filename rawhide uses.
# The CHECKSUM filename embeds the compose and the word `images`, so it
# does not match the stable path's pattern; the script does not derive
# it from the image, it parses both out of the index side by side.
IMG_RAWHIDE=Fedora-Cloud-Base-Generic-Rawhide-20261002.n.0.x86_64.qcow2
CS_RAWHIDE=Fedora-Cloud-images-Rawhide-x86_64-20261002.n.0-CHECKSUM

# Build a synthetic image body and the CHECKSUM line for it. The stub
# serves the body to curl, the script hashes the body and compares against
# what the stub's CHECKSUM claims. This keeps every assertion inside the
# test self-consistent and avoids depending on the real Fedora image
# bytes (which are ~500 MB and out of scope for a unit test).
IMG_BODY="$(printf '%.0s;' $(seq 1 2048))"
IMG_HASH="$(printf '%s' "$IMG_BODY" | sha256sum | awk '{print $1}')"

# --- common run helper --------------------------------------------------------
#
# Run the script with the stub on PATH. Echoes a delimiter-separated
# record: <rc> on its own line, then stdout up to `---`, then stderr up
# to `---`. The caller slices what it needs with `sed`/`awk`.
run_script() { # <major> <out>
	local rc
	local out
	out="$(STUB_ROOT="$ROOT" STUB_COUNTERS_DIR="$COUNTERS" \
		STUB_LIMITS_DIR="$LIMITS" \
		PATH="$STUB_DIR:$PATH" FETCH_SLEEP=0 \
		bash "$script" "$1" "$2" 2> "$WORK/runner.err")"
	rc=$?
	printf '%d\n%s\n---\n' "$rc" "$out"
	cat "$WORK/runner.err" 2>/dev/null || true
	printf -- '---\n'
}

plant_stable_release() { # <root> <expected-sha> <body>
	local dir="$1"
	local expected="$2"
	local body="$3"
	mkdir -p "$dir"
	cat > "$dir/index.html" <<EOF
<a href="$IMG_44">$IMG_44</a>
<a href="$CS_44">$CS_44</a>
EOF
	cat > "$dir/$CS_44" <<CHECKSUM
-----BEGIN PGP SIGNED MESSAGE-----
Hash: SHA256

SHA256 ($IMG_44) = $expected
-----BEGIN PGP SIGNATURE-----
stub
-----END PGP SIGNATURE-----
CHECKSUM
	printf '%s' "$body" > "$dir/$IMG_44"
}

plant_rawhide() { # <root> <expected-sha> <body>
	local dir="$1"
	local expected="$2"
	local body="$3"
	mkdir -p "$dir"
	cat > "$dir/index.html" <<EOF
<a href="$IMG_RAWHIDE">$IMG_RAWHIDE</a>
<a href="$CS_RAWHIDE">$CS_RAWHIDE</a>
EOF
	cat > "$dir/$CS_RAWHIDE" <<CHECKSUM
-----BEGIN PGP SIGNED MESSAGE-----
Hash: SHA256

SHA256 ($IMG_RAWHIDE) = $expected
-----BEGIN PGP SIGNATURE-----
stub
-----END PGP SIGNATURE-----
CHECKSUM
	printf '%s' "$body" > "$dir/$IMG_RAWHIDE"
}

# Drop a per-(host, path) 404 limit file the stub consumes to decide
# whether THIS call is the first N of a path the host is supposed to
# answer 404 for. The path is the part after the host; the file lives at
# $LIMITS/<host>/<path> with content = count.
plant_path_limit() { # <host> <path> <count>
	local host="$1"
	local path="$2"
	local count="$3"
	mkdir -p "$LIMITS/$host/$(dirname -- "$path")"
	printf '%s' "$count" > "$LIMITS/$host/$path"
}

# Stable path the script asks for; the CHECKSUM path is the file name
# appended on top of it. Used by the per-path 404 limit helper to plant
# a limit on the CHECKSUM request only.
STABLE_IMAGES="/pub/fedora/linux/releases/44/Cloud/x86_64/images"

# Assertion: the per-host call log for `host` contains zero entries
# matching the given substring. Used to assert that the CHECKSUM was
# never fetched from the fallback.
assert_no_path_in_log() { # <description> <host> <substring>
	local desc="$1"
	local host="$2"
	local needle="$3"
	local log="$COUNTERS/$host.log"
	if [ ! -f "$log" ] || ! grep -qF "$needle" "$log"; then
		echo "ok    $desc"
		pass=$((pass + 1))
	else
		echo "FAIL  $desc"
		echo "        $log contained:"
		grep -F "$needle" "$log" | sed 's/^/          /'
		fail=$((fail + 1))
	fi
}

# ---------------------------------------------------------------------------
# Case 1: primary 404 for the index twice, then 200.
# ---------------------------------------------------------------------------
setup
STUB_404_dl_fedoraproject_org=2
stable_root="$ROOT/dl.fedoraproject.org/pub/fedora/linux/releases/44/Cloud/x86_64/images"
plant_stable_release "$stable_root" "$IMG_HASH" "$IMG_BODY"
# Mirror the same content on the fallback so the test still passes if the
# stub ever reaches the fallback host. Same expected hash, same body.
fallback_root="$ROOT/download.fedoraproject.org/pub/fedora/linux/releases/44/Cloud/x86_64/images"
plant_stable_release "$fallback_root" "$IMG_HASH" "$IMG_BODY"

RESULT="$(run_script 5 "$WORK/vm.qcow2")"
RC="$(printf '%s\n' "$RESULT" | sed -n 1p)"
STDOUT="$(printf '%s\n' "$RESULT" | sed -n '2,/^---$/p' | sed '$d')"

check "case1: script exits 0" "0" "$RC"
check "case1: stdout prints the resolved image name" "$IMG_44" "$STDOUT"
check "case1: output file is non-empty" \
	"1" "$([ -s "$WORK/vm.qcow2" ] && echo 1 || echo 0)"
assert_no_path_in_log "case1: CHECKSUM never reached the fallback" \
	"download.fedoraproject.org" "$CS_44"

teardown

# ---------------------------------------------------------------------------
# Case 2: primary always 404. The CHECKSUM is primary-only, so the
# CHECKSUM fetch exhausts the retry budget and the script fails with the
# primary-source CHECKSUM error. (The image is never touched.)
# ---------------------------------------------------------------------------
setup
STUB_404_dl_fedoraproject_org=999
stable_root="$ROOT/dl.fedoraproject.org/pub/fedora/linux/releases/44/Cloud/x86_64/images"
# Plant no files on the primary; every primary call is a 404.
fallback_root="$ROOT/download.fedoraproject.org/pub/fedora/linux/releases/44/Cloud/x86_64/images"
plant_stable_release "$fallback_root" "$IMG_HASH" "$IMG_BODY"

RESULT="$(run_script 5 "$WORK/vm.qcow2")"
RC="$(printf '%s\n' "$RESULT" | sed -n 1p)"
STDERR="$(printf '%s\n' "$RESULT" | awk '/^---$/{f++; next} f==1{print}')"
STDOUT="$(printf '%s\n' "$RESULT" | sed -n '2,/^---$/p' | sed '$d')"

check "case2: script exits non-zero" "1" "$RC"
check "case2: stdout is empty" "" "$STDOUT"
check "case2: stderr names the CHECKSUM" "1" \
	"$(printf '%s' "$STDERR" | grep -q "$CS_44" && echo 1 || echo 0)"
check "case2: stderr names the primary URL" "1" \
	"$(printf '%s' "$STDERR" | grep -q 'dl.fedoraproject.org' && echo 1 || echo 0)"
check "case2: stderr names 'could not fetch'" "1" \
	"$(printf '%s' "$STDERR" | grep -q 'could not fetch' && echo 1 || echo 0)"
assert_no_path_in_log "case2: CHECKSUM never reached the fallback" \
	"download.fedoraproject.org" "$CS_44"

teardown

# ---------------------------------------------------------------------------
# Case 3: both hosts always 404. Resolve loop runs out of tries.
# ---------------------------------------------------------------------------
setup
STUB_404_dl_fedoraproject_org=999
STUB_404_download_fedoraproject_org=999
# No files in the roots -- every fetch is a 404.
RESULT="$(run_script 5 "$WORK/vm.qcow2")"
RC="$(printf '%s\n' "$RESULT" | sed -n 1p)"
STDERR="$(printf '%s\n' "$RESULT" | awk '/^---$/{f++; next} f==1{print}')"
STDOUT="$(printf '%s\n' "$RESULT" | sed -n '2,/^---$/p' | sed '$d')"

check "case3: script exits non-zero" "1" "$RC"
check "case3: stdout is empty" "" "$STDOUT"
check "case3: stderr names 'could not resolve'" "1" \
	"$(printf '%s' "$STDERR" | grep -q "could not resolve" && echo 1 || echo 0)"

teardown

# ---------------------------------------------------------------------------
# Case 4: primary serves the index but 404s the CHECKSUM on every call
# (file not planted on primary, so the stub returns 22 regardless of the
# per-host limit). The fallback has the image and a CHECKSUM matching it,
# but the CHECKSUM is primary-only, so the script must fail with the
# primary-source CHECKSUM error and must not leave the image file
# behind -- the script exits before phase 3.
# ---------------------------------------------------------------------------
setup
STUB_404_dl_fedoraproject_org=0
stable_root="$ROOT/dl.fedoraproject.org/pub/fedora/linux/releases/44/Cloud/x86_64/images"
# Plant the index and image on primary; deliberately DO NOT plant the
# CHECKSUM. Every primary request for the CHECKSUM lands on a missing
# file, so the stub returns 22 for it.
mkdir -p "$stable_root"
cat > "$stable_root/index.html" <<EOF
<a href="$IMG_44">$IMG_44</a>
<a href="$CS_44">$CS_44</a>
EOF
printf '%s' "$IMG_BODY" > "$stable_root/$IMG_44"
fallback_root="$ROOT/download.fedoraproject.org/pub/fedora/linux/releases/44/Cloud/x86_64/images"
plant_stable_release "$fallback_root" "$IMG_HASH" "$IMG_BODY"

RESULT="$(run_script 5 "$WORK/vm.qcow2")"
RC="$(printf '%s\n' "$RESULT" | sed -n 1p)"
STDERR="$(printf '%s\n' "$RESULT" | awk '/^---$/{f++; next} f==1{print}')"
STDOUT="$(printf '%s\n' "$RESULT" | sed -n '2,/^---$/p' | sed '$d')"

check "case4: script exits non-zero" "1" "$RC"
check "case4: stdout is empty" "" "$STDOUT"
check "case4: stderr names the CHECKSUM" "1" \
	"$(printf '%s' "$STDERR" | grep -q "$CS_44" && echo 1 || echo 0)"
check "case4: stderr names 'could not fetch'" "1" \
	"$(printf '%s' "$STDERR" | grep -q 'could not fetch' && echo 1 || echo 0)"
check "case4: output file does not exist afterwards" \
	"0" "$([ -e "$WORK/vm.qcow2" ] && echo 1 || echo 0)"
assert_no_path_in_log "case4: CHECKSUM never reached the fallback" \
	"download.fedoraproject.org" "$CS_44"

teardown

# ---------------------------------------------------------------------------
# Case 5: primary 404s for the CHECKSUM twice, then serves it. The index
# and image serve on the first call (per-host limit 0), so the only
# retries are for the CHECKSUM, exercised through the per-path 404 limit
# file. Must succeed.
# ---------------------------------------------------------------------------
setup
STUB_404_dl_fedoraproject_org=0
stable_root="$ROOT/dl.fedoraproject.org/pub/fedora/linux/releases/44/Cloud/x86_64/images"
plant_stable_release "$stable_root" "$IMG_HASH" "$IMG_BODY"
# Per-path 404 limit on the primary for the CHECKSUM: the first two
# attempts 404, the third (and after) serve. Index + image have no
# per-path limit, so they use the per-host limit (serve immediately).
plant_path_limit "dl.fedoraproject.org" \
	"${STABLE_IMAGES}/${CS_44}" 2
fallback_root="$ROOT/download.fedoraproject.org/pub/fedora/linux/releases/44/Cloud/x86_64/images"
plant_stable_release "$fallback_root" "$IMG_HASH" "$IMG_BODY"

RESULT="$(run_script 5 "$WORK/vm.qcow2")"
RC="$(printf '%s\n' "$RESULT" | sed -n 1p)"
STDOUT="$(printf '%s\n' "$RESULT" | sed -n '2,/^---$/p' | sed '$d')"

check "case5: script exits 0" "0" "$RC"
check "case5: stdout prints the resolved image name" "$IMG_44" "$STDOUT"
check "case5: output file is non-empty" \
	"1" "$([ -s "$WORK/vm.qcow2" ] && echo 1 || echo 0)"
assert_no_path_in_log "case5: CHECKSUM never reached the fallback" \
	"download.fedoraproject.org" "$CS_44"

teardown

# ---------------------------------------------------------------------------
# Case 6: checksum mismatch. Image is downloaded but the SHA-256 of the
# downloaded bytes does not match what the CHECKSUM file claims. The
# script must delete the output and fail with both hashes named.
# ---------------------------------------------------------------------------
setup
# A different valid-shape 64-hex-character SHA for the CHECKSUM to claim, so
# the script takes the mismatch branch (not the "no entry" branch). Hex
# chars are required because the script's regex pins the digest to
# [0-9a-f]{64} to guard against a CHECKSUM line that looks like a SHA but
# has been corrupted in transit.
WRONG_EXPECTED="$(printf 'a%.0s' $(seq 1 64))"
STUB_404_dl_fedoraproject_org=0
stable_root="$ROOT/dl.fedoraproject.org/pub/fedora/linux/releases/44/Cloud/x86_64/images"
plant_stable_release "$stable_root" "$WRONG_EXPECTED" "$IMG_BODY"
fallback_root="$ROOT/download.fedoraproject.org/pub/fedora/linux/releases/44/Cloud/x86_64/images"
plant_stable_release "$fallback_root" "$WRONG_EXPECTED" "$IMG_BODY"

RESULT="$(run_script 5 "$WORK/vm.qcow2")"
RC="$(printf '%s\n' "$RESULT" | sed -n 1p)"
STDERR="$(printf '%s\n' "$RESULT" | awk '/^---$/{f++; next} f==1{print}')"
STDOUT="$(printf '%s\n' "$RESULT" | sed -n '2,/^---$/p' | sed '$d')"

check "case6: script exits non-zero" "1" "$RC"
check "case6: stdout is empty" "" "$STDOUT"
check "case6: stderr names the expected SHA-256" "1" \
	"$(printf '%s' "$STDERR" | grep -q "$WRONG_EXPECTED" && echo 1 || echo 0)"
check "case6: stderr names the actual SHA-256" "1" \
	"$(printf '%s' "$STDERR" | grep -q "$IMG_HASH" && echo 1 || echo 0)"
check "case6: output file does not exist afterwards" \
	"0" "$([ -e "$WORK/vm.qcow2" ] && echo 1 || echo 0)"
assert_no_path_in_log "case6: CHECKSUM never reached the fallback" \
	"download.fedoraproject.org" "$CS_44"

teardown

# ---------------------------------------------------------------------------
# Case 7: the major picks the right directory and the right image pattern.
# Major 5 -> releases/44 + Generic-44-...; major 6 -> rawhide.
# ---------------------------------------------------------------------------
setup
STUB_404_dl_fedoraproject_org=0
stable_root="$ROOT/dl.fedoraproject.org/pub/fedora/linux/releases/44/Cloud/x86_64/images"
rawhide_root="$ROOT/dl.fedoraproject.org/pub/fedora/linux/development/rawhide/Cloud/x86_64/images"
plant_stable_release "$stable_root" "$IMG_HASH" "$IMG_BODY"
plant_rawhide "$rawhide_root" "$IMG_HASH" "$IMG_BODY"

# Run with major 5; the script must pick the 44 image, not the rawhide
# one. We do not plant the rawhide index under releases/44, so the regex
# for major 5 only sees the stable image and the resolve hits it.
RESULT="$(run_script 5 "$WORK/vm-44.qcow2")"
RC="$(printf '%s\n' "$RESULT" | sed -n 1p)"
STDOUT_5="$(printf '%s\n' "$RESULT" | sed -n '2,/^---$/p' | sed '$d')"
check "case7a: major 5 exits 0" "0" "$RC"
check "case7a: major 5 resolves Generic-44" "1" \
	"$(printf '%s' "$STDOUT_5" | grep -q "Generic-44" && echo 1 || echo 0)"

# Run with major 6; must pick the rawhide image. The stable path's index
# is also present, but the major-6 pattern only matches Generic-Rawhide.
RESULT="$(run_script 6 "$WORK/vm-rawhide.qcow2")"
RC="$(printf '%s\n' "$RESULT" | sed -n 1p)"
STDOUT_6="$(printf '%s\n' "$RESULT" | sed -n '2,/^---$/p' | sed '$d')"
check "case7b: major 6 exits 0" "0" "$RC"
check "case7b: major 6 resolves Generic-Rawhide" "1" \
	"$(printf '%s' "$STDOUT_6" | grep -q "Generic-Rawhide" && echo 1 || echo 0)"
assert_no_path_in_log "case7b: rawhide CHECKSUM never reached the fallback" \
	"download.fedoraproject.org" "$CS_RAWHIDE"

teardown

echo
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]