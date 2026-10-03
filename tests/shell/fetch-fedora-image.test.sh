#!/usr/bin/env bash
#
# Behaviour tests for `.github/scripts/fetch-fedora-image.sh`.
#
# The lane pulls ~500 MB off dl.fedoraproject.org, which 404s about
# half the time in streaks longer than `--retry 5 --retry-delay 5`
# covers. The script retries up to 8 times, falls back to the mirror
# redirector download.fedoraproject.org for the index and the image,
# fetches the CHECKSUM from the primary only, and verifies the SHA-256
# of what came down against the value the CHECKSUM carries. None of
# that has a real network in the loop. The stub curl in
# fetch-fedora-image.stub.sh serves them from a temp tree per host
# and answers 404 for the first N calls to a host (configurable per
# case), 200 from the temp tree thereafter.
#
# Cases:
#   - primary 404 for the index twice, then 200: resolves and downloads
#   - primary always 404: CHECKSUM phase fails (primary-only)
#   - both always 404: fails with the `could not resolve` error
#   - primary serves the index but 404s the CHECKSUM on every call,
#     fallback has the image and a CHECKSUM matching it: fails with
#     the primary-only CHECKSUM error, no image file left behind
#   - primary 404s the CHECKSUM twice, then serves it: succeeds
#   - checksum mismatch: fails, the error names both hashes, output gone
#   - the major selects the right path and name pattern (5 for 44, 6
#     for rawhide)
#
# The records for the defects fixed in this branch (partial CHECKSUM
# body, strict parser, missing-CHECKSUM index, no-redirect CHECKSUM,
# overall deadline) live in fetch-fedora-image-records.test.sh.
#
# FETCH_SLEEP=0 is set so the resolve loop does not sleep between
# tries; the worst-case run here is a handful of milliseconds.
#
# Requires: bash, coreutils, sha256sum, the script under test, nothing
# else.
set -u

# shellcheck source=tests/shell/fetch-fedora-image.stub.sh
. "$(dirname "$0")/fetch-fedora-image.stub.sh"

# Per-test planters. The stable and rawhide releases plant a matching
# CHECKSUM and image, so the tests that exercise the happy path can
# just call them and the index/CHECKSUM/image land on the same
# content root.
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

# ---------------------------------------------------------------------------
# Case 1: primary 404 for the index twice, then 200.
# ---------------------------------------------------------------------------
setup
STUB_404_dl_fedoraproject_org=2
stable_root="$ROOT/dl.fedoraproject.org/pub/fedora/linux/releases/44/Cloud/x86_64/images"
plant_stable_release "$stable_root" "$IMG_HASH" "$IMG_BODY"
# Mirror the same content on the fallback so the test still passes
# if the stub ever reaches the fallback host. Same expected hash,
# same body.
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
# CHECKSUM fetch exhausts the retry budget and the script fails with
# the primary-source CHECKSUM error. (The image is never touched.)
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
# Case 4: primary serves the index but 404s the CHECKSUM on every
# call (file not planted on primary, so the stub returns 22
# regardless of the per-host limit). The fallback has the image and
# a CHECKSUM matching it, but the CHECKSUM is primary-only, so the
# script must fail with the primary-source CHECKSUM error and must
# not leave the image file behind -- the script exits before phase
# 3.
# ---------------------------------------------------------------------------
setup
STUB_404_dl_fedoraproject_org=0
stable_root="$ROOT/dl.fedoraproject.org/pub/fedora/linux/releases/44/Cloud/x86_64/images"
# Plant the index and image on primary; deliberately DO NOT plant
# the CHECKSUM. Every primary request for the CHECKSUM lands on a
# missing file, so the stub returns 22 for it.
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
# Case 5: primary 404s for the CHECKSUM twice, then serves it. The
# index and image serve on the first call (per-host limit 0), so the
# only retries are for the CHECKSUM, exercised through the per-path
# 404 limit file. Must succeed.
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
# Case 6: checksum mismatch. Image is downloaded but the SHA-256 of
# the downloaded bytes does not match what the CHECKSUM file
# claims. The script must delete the output and fail with both
# hashes named.
# ---------------------------------------------------------------------------
setup
# A different valid-shape 64-hex-character SHA for the CHECKSUM to
# claim, so the script takes the mismatch branch (not the "no entry"
# branch). Hex chars are required because the script's regex pins
# the digest to [0-9a-f]{64} to guard against a CHECKSUM line that
# looks like a SHA but has been corrupted in transit.
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
# Case 7: the major picks the right directory and the right image
# pattern. Major 5 -> releases/44 + Generic-44-...; major 6 ->
# rawhide.
# ---------------------------------------------------------------------------
setup
STUB_404_dl_fedoraproject_org=0
stable_root="$ROOT/dl.fedoraproject.org/pub/fedora/linux/releases/44/Cloud/x86_64/images"
rawhide_root="$ROOT/dl.fedoraproject.org/pub/fedora/linux/development/rawhide/Cloud/x86_64/images"
plant_stable_release "$stable_root" "$IMG_HASH" "$IMG_BODY"
plant_rawhide "$rawhide_root" "$IMG_HASH" "$IMG_BODY"

# Run with major 5; the script must pick the 44 image, not the
# rawhide one. We do not plant the rawhide index under releases/44,
# so the regex for major 5 only sees the stable image and the
# resolve hits it.
RESULT="$(run_script 5 "$WORK/vm-44.qcow2")"
RC="$(printf '%s\n' "$RESULT" | sed -n 1p)"
STDOUT_5="$(printf '%s\n' "$RESULT" | sed -n '2,/^---$/p' | sed '$d')"
check "case7a: major 5 exits 0" "0" "$RC"
check "case7a: major 5 resolves Generic-44" "1" \
	"$(printf '%s' "$STDOUT_5" | grep -q "Generic-44" && echo 1 || echo 0)"

# Run with major 6; must pick the rawhide image. The stable
# path's index is also present, but the major-6 pattern only
# matches Generic-Rawhide.
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
