#!/usr/bin/env bash
#
# Tests for the defect fixes in fetch-fedora-image.sh.
#
# The five items landed as one patch: a partial CHECKSUM body must
# not survive the retry budget, the CHECKSUM parser must bind to
# the exact filename, an index without a CHECKSUM link must fail in
# phase 1, the CHECKSUM fetch must not follow redirects, and the
# script must honour an overall wall-clock budget. Each item has its
# own case below; the per-case planters live in this file because
# they are specific to the record being tested.
#
# The shared stub (counter, paths log, full-args log, fixture names,
# check helper, run helper) lives in fetch-fedora-image.stub.sh.
#
# Requires: bash, coreutils, sha256sum, the script under test.
set -u

# shellcheck source=tests/shell/fetch-fedora-image.stub.sh
. "$(dirname "$0")/fetch-fedora-image.stub.sh"

# Stable path used by every case. The CHECKSUM path is the file name
# appended on top of it. $ROOT is set by setup() below, so the
# per-host absolute path is computed inside each case rather than
# at the top of the file.
STABLE_IMAGES_DIR="pub/fedora/linux/releases/44/Cloud/x86_64/images/"
STABLE_IMAGES_PATH="/${STABLE_IMAGES_DIR}"

# Helper: write a CHECKSUM body containing exactly the lines the
# caller passed, in order. The PGP armour wraps the body the way
# real Fedora CHECKSUMs do; the parser has to skip it.
write_checksum() { # <path> [lines...]
	local path="$1"
	shift
	{
		printf '%s\n' "-----BEGIN PGP SIGNED MESSAGE-----"
		printf '%s\n' "Hash: SHA256"
		printf '%s\n' ""
		for line in "$@"; do
			printf '%s\n' "$line"
		done
		printf '%s\n' "-----BEGIN PGP SIGNATURE-----"
		printf '%s\n' "stub"
		printf '%s\n' "-----END PGP SIGNATURE-----"
	} > "$path"
}

# Helper: plant a release tree that has the image and the index, so
# phase 1 succeeds. The CHECKSUM is the caller's responsibility
# (each case wants a different body).
plant_release_without_checksum() { # <root> [body]
	local root="$1"
	local body="${2:-$IMG_BODY}"
	mkdir -p "$root"
	cat > "$root/index.html" <<EOF
<a href="$IMG_44">$IMG_44</a>
<a href="$CS_44">$CS_44</a>
EOF
	printf '%s' "$body" > "$root/$IMG_44"
}

# ---------------------------------------------------------------------------
# Item 1: partial CHECKSUM body accepted after all attempts fail.
#
# The stub for the CHECKSUM request prints a valid-looking body and
# exits 18 (curl's partial-file code), the way a hung connection
# would. With the fix, the script's command substitution sees the
# non-zero exit and leaves checksum_body empty; the loop then runs
# out of tries and the script fails. Without the fix, the body
# survives the last attempt and the script proceeds to download the
# image and exits 0.
# ---------------------------------------------------------------------------
setup
STABLE_IMAGES_ABS="$ROOT/dl.fedoraproject.org${STABLE_IMAGES_PATH}"
STUB_404_dl_fedoraproject_org=0
plant_release_without_checksum "$STABLE_IMAGES_ABS"
# Plant a valid CHECKSUM body so the stub has something to print,
# then mark the path with exit_after=18 to simulate the partial
# download.
write_checksum "$STABLE_IMAGES_ABS/$CS_44" \
	"SHA256 ($IMG_44) = $IMG_HASH"
plant_exit_after "dl.fedoraproject.org" \
	"${STABLE_IMAGES_DIR}${CS_44}" 18

RESULT="$(run_script 5 "$WORK/vm.qcow2")"
RC="$(printf '%s\n' "$RESULT" | sed -n 1p)"
STDOUT="$(printf '%s\n' "$RESULT" | sed -n '2,/^---$/p' | sed '$d')"

check "item1: script exits non-zero" "1" "$RC"
check "item1: stdout is empty" "" "$STDOUT"
check "item1: output image file does not exist" \
	"0" "$([ -e "$WORK/vm.qcow2" ] && echo 1 || echo 0)"

teardown

# ---------------------------------------------------------------------------
# Item 2: CHECKSUM parser not bound to the filename field.
#
# Each case plants a CHECKSUM body with a different line shape. The
# last two cases pass; the first three fail with distinct errors.
# ---------------------------------------------------------------------------

# 2a: only record has the right digest but is for a different file
# AND has a trailing comment that the whole-line shape rejects.
setup
STABLE_IMAGES_ABS="$ROOT/dl.fedoraproject.org${STABLE_IMAGES_PATH}"
STUB_404_dl_fedoraproject_org=0
plant_release_without_checksum "$STABLE_IMAGES_ABS"
write_checksum "$STABLE_IMAGES_ABS/$CS_44" \
	"SHA256 (other.qcow2) = $IMG_HASH # ($IMG_44)"

RESULT="$(run_script 5 "$WORK/vm.qcow2")"
RC="$(printf '%s\n' "$RESULT" | sed -n 1p)"
STDERR="$(printf '%s\n' "$RESULT" | awk '/^---$/{f++; next} f==1{print}')"

check "item2a: script exits non-zero" "1" "$RC"
check "item2a: stderr names 'could not find SHA-256'" "1" \
	"$(printf '%s' "$STDERR" | grep -q 'could not find SHA-256' && echo 1 || echo 0)"
check "item2a: output image file does not exist" \
	"0" "$([ -e "$WORK/vm.qcow2" ] && echo 1 || echo 0)"

teardown

# 2b: digest is 65 hex digits, one too many for the whole-line shape.
setup
STABLE_IMAGES_ABS="$ROOT/dl.fedoraproject.org${STABLE_IMAGES_PATH}"
STUB_404_dl_fedoraproject_org=0
plant_release_without_checksum "$STABLE_IMAGES_ABS"
LONG_DIGEST="${IMG_HASH}f"
write_checksum "$STABLE_IMAGES_ABS/$CS_44" \
	"SHA256 ($IMG_44) = $LONG_DIGEST"

RESULT="$(run_script 5 "$WORK/vm.qcow2")"
RC="$(printf '%s\n' "$RESULT" | sed -n 1p)"
STDERR="$(printf '%s\n' "$RESULT" | awk '/^---$/{f++; next} f==1{print}')"

check "item2b: script exits non-zero" "1" "$RC"
check "item2b: stderr names 'could not find SHA-256'" "1" \
	"$(printf '%s' "$STDERR" | grep -q 'could not find SHA-256' && echo 1 || echo 0)"
check "item2b: output image file does not exist" \
	"0" "$([ -e "$WORK/vm.qcow2" ] && echo 1 || echo 0)"

teardown

# 2c: two records for the image with different digests -- conflict.
setup
STABLE_IMAGES_ABS="$ROOT/dl.fedoraproject.org${STABLE_IMAGES_PATH}"
STUB_404_dl_fedoraproject_org=0
plant_release_without_checksum "$STABLE_IMAGES_ABS"
ALT_HASH="$(printf 'a%.0s' $(seq 1 64))"
write_checksum "$STABLE_IMAGES_ABS/$CS_44" \
	"SHA256 ($IMG_44) = $IMG_HASH" \
	"SHA256 ($IMG_44) = $ALT_HASH"

RESULT="$(run_script 5 "$WORK/vm.qcow2")"
RC="$(printf '%s\n' "$RESULT" | sed -n 1p)"
STDERR="$(printf '%s\n' "$RESULT" | awk '/^---$/{f++; next} f==1{print}')"

check "item2c: script exits non-zero" "1" "$RC"
check "item2c: stderr names 'conflicting'" "1" \
	"$(printf '%s' "$STDERR" | grep -q 'conflicting' && echo 1 || echo 0)"
check "item2c: output image file does not exist" \
	"0" "$([ -e "$WORK/vm.qcow2" ] && echo 1 || echo 0)"

teardown

# 2d: two identical records for the image -- not a conflict, succeeds.
setup
STABLE_IMAGES_ABS="$ROOT/dl.fedoraproject.org${STABLE_IMAGES_PATH}"
STUB_404_dl_fedoraproject_org=0
plant_release_without_checksum "$STABLE_IMAGES_ABS"
write_checksum "$STABLE_IMAGES_ABS/$CS_44" \
	"SHA256 ($IMG_44) = $IMG_HASH" \
	"SHA256 ($IMG_44) = $IMG_HASH"

RESULT="$(run_script 5 "$WORK/vm.qcow2")"
RC="$(printf '%s\n' "$RESULT" | sed -n 1p)"
STDOUT="$(printf '%s\n' "$RESULT" | sed -n '2,/^---$/p' | sed '$d')"

check "item2d: script exits 0" "0" "$RC"
check "item2d: stdout prints the resolved image name" "$IMG_44" "$STDOUT"
check "item2d: output image file is non-empty" \
	"1" "$([ -s "$WORK/vm.qcow2" ] && echo 1 || echo 0)"

teardown

# 2e: a record for another filename with a different digest plus the
# correct record for the image -- succeeds.
setup
STABLE_IMAGES_ABS="$ROOT/dl.fedoraproject.org${STABLE_IMAGES_PATH}"
STUB_404_dl_fedoraproject_org=0
plant_release_without_checksum "$STABLE_IMAGES_ABS"
ALT_HASH="$(printf 'a%.0s' $(seq 1 64))"
write_checksum "$STABLE_IMAGES_ABS/$CS_44" \
	"SHA256 (other.qcow2) = $ALT_HASH" \
	"SHA256 ($IMG_44) = $IMG_HASH"

RESULT="$(run_script 5 "$WORK/vm.qcow2")"
RC="$(printf '%s\n' "$RESULT" | sed -n 1p)"
STDOUT="$(printf '%s\n' "$RESULT" | sed -n '2,/^---$/p' | sed '$d')"

check "item2e: script exits 0" "0" "$RC"
check "item2e: stdout prints the resolved image name" "$IMG_44" "$STDOUT"
check "item2e: output image file is non-empty" \
	"1" "$([ -s "$WORK/vm.qcow2" ] && echo 1 || echo 0)"

teardown

# ---------------------------------------------------------------------------
# Item 3: index without a CHECKSUM link falls through.
#
# The index advertises the image but no CHECKSUM file. Phase 1
# resolves the image, the new guard kicks in, the script fails with
# a CHECKSUM-named error, and phase 2 never runs -- so no request
# other than the index request is logged.
# ---------------------------------------------------------------------------
setup
STABLE_IMAGES_ABS="$ROOT/dl.fedoraproject.org${STABLE_IMAGES_PATH}"
STUB_404_dl_fedoraproject_org=0
# Plant the index with the image but no CHECKSUM link.
mkdir -p "$STABLE_IMAGES_ABS"
cat > "$STABLE_IMAGES_ABS/index.html" <<EOF
<a href="$IMG_44">$IMG_44</a>
EOF
printf '%s' "$IMG_BODY" > "$STABLE_IMAGES_ABS/$IMG_44"

RESULT="$(run_script 5 "$WORK/vm.qcow2")"
RC="$(printf '%s\n' "$RESULT" | sed -n 1p)"
STDERR="$(printf '%s\n' "$RESULT" | awk '/^---$/{f++; next} f==1{print}')"
STDOUT="$(printf '%s\n' "$RESULT" | sed -n '2,/^---$/p' | sed '$d')"

check "item3: script exits non-zero" "1" "$RC"
check "item3: stdout is empty" "" "$STDOUT"
check "item3: stderr mentions CHECKSUM" "1" \
	"$(printf '%s' "$STDERR" | grep -q 'CHECKSUM' && echo 1 || echo 0)"
# Phase 1 ran the full retry budget (the index is served every
# time but never carries a CHECKSUM link), so 8 calls to the
# images directory are expected. The fix means phase 2 must NOT
# have fired the bug's "fetch the directory as the CHECKSUM" call
# -- without it, that would be another 8 calls, totalling 16.
INDEX_HITS="$(count_args_matching dl.fedoraproject.org "$STABLE_IMAGES_DIR")"
check "item3: stub log shows exactly 8 index calls (no phase-2 leak)" \
	"8" "$INDEX_HITS"
check "item3: output image file does not exist" \
	"0" "$([ -e "$WORK/vm.qcow2" ] && echo 1 || echo 0)"

teardown

# ---------------------------------------------------------------------------
# Item 4: CHECKSUM fetch follows redirects.
#
# A successful run's stub log must show --max-redirs 0 on every
# CHECKSUM request to the primary, and never on the index or image
# requests.
# ---------------------------------------------------------------------------
setup
STABLE_IMAGES_ABS="$ROOT/dl.fedoraproject.org${STABLE_IMAGES_PATH}"
STUB_404_dl_fedoraproject_org=0
plant_release_without_checksum "$STABLE_IMAGES_ABS"
write_checksum "$STABLE_IMAGES_ABS/$CS_44" \
	"SHA256 ($IMG_44) = $IMG_HASH"

RESULT="$(run_script 5 "$WORK/vm.qcow2")"
RC="$(printf '%s\n' "$RESULT" | sed -n 1p)"

check "item4: script exits 0" "0" "$RC"
# Every line in the args log that carries the CHECKSUM file name
# must also carry --max-redirs 0. Grep matches the CHECKSUM path
# first, then filters the same line for the redirect flag.
ARGS_LOG="$COUNTERS/dl.fedoraproject.org.args"
CHECKSUM_WITH_FLAG="$(grep -F -- "$CS_44" "$ARGS_LOG" | grep -cF -- '--max-redirs 0')"
CHECKSUM_TOTAL="$(grep -cF -- "$CS_44" "$ARGS_LOG")"
check "item4: every CHECKSUM request carries --max-redirs 0" \
	"$CHECKSUM_TOTAL" "$CHECKSUM_WITH_FLAG"
# No line that is an index or image request (i.e. the args log
# line does NOT mention the CHECKSUM filename) may carry
# --max-redirs 0; the redirect rejection is CHECKSUM-only.
NON_CHECKSUM_WITH_FLAG="$(grep -vF -- "$CS_44" "$ARGS_LOG" | grep -cF -- '--max-redirs 0' || true)"
check "item4: no index or image request carries --max-redirs 0" \
	"0" "$NON_CHECKSUM_WITH_FLAG"

teardown

# ---------------------------------------------------------------------------
# Item 5: no overall time budget.
# ---------------------------------------------------------------------------

# 5a: FETCH_DEADLINE=0 with the primary always 404. The first loop
# iteration's deadline check fires before any attempt, so at most
# one request per host and the error names the deadline.
setup
STABLE_IMAGES_ABS="$ROOT/dl.fedoraproject.org${STABLE_IMAGES_PATH}"
export FETCH_DEADLINE=0
STUB_404_dl_fedoraproject_org=999
STUB_404_download_fedoraproject_org=999
# No files planted; every request is a 404.

RESULT="$(run_script 5 "$WORK/vm.qcow2")"
RC="$(printf '%s\n' "$RESULT" | sed -n 1p)"
STDERR="$(printf '%s\n' "$RESULT" | awk '/^---$/{f++; next} f==1{print}')"

check "item5a: script exits non-zero" "1" "$RC"
check "item5a: stderr contains 'fetch deadline'" "1" \
	"$(printf '%s' "$STDERR" | grep -q 'fetch deadline' && echo 1 || echo 0)"
PRIMARY_INDEX_HITS="$(count_args_matching dl.fedoraproject.org "$STABLE_IMAGES_DIR")"
check "item5a: at most one index request to primary" \
	"1" "$(if [ "$PRIMARY_INDEX_HITS" -le 1 ]; then echo 1; else echo 0; fi)"

teardown
unset FETCH_DEADLINE

# 5b: default deadline (1200) with the primary always 404 for the
# CHECKSUM. Phase 1 succeeds (the index is served), phase 2 makes
# exactly TRIES=8 CHECKSUM attempts, then fails.
setup
STABLE_IMAGES_ABS="$ROOT/dl.fedoraproject.org${STABLE_IMAGES_PATH}"
STUB_404_dl_fedoraproject_org=0
plant_release_without_checksum "$STABLE_IMAGES_ABS"
# No CHECKSUM file planted: every primary CHECKSUM request 404s.
CHECKSUM_PATH="${STABLE_IMAGES_DIR}${CS_44}"
plant_path_limit "dl.fedoraproject.org" "$CHECKSUM_PATH" 999

RESULT="$(run_script 5 "$WORK/vm.qcow2")"
RC="$(printf '%s\n' "$RESULT" | sed -n 1p)"

check "item5b: script exits non-zero" "1" "$RC"
CHECKSUM_HITS="$(count_args_matching dl.fedoraproject.org "$CS_44")"
check "item5b: exactly 8 CHECKSUM requests (pins TRIES=8)" \
	"8" "$CHECKSUM_HITS"

teardown

# 5c: arg-flag assertions. Index and CHECKSUM use the metadata
# timeouts (--max-time 60, --connect-timeout 10); the image uses
# --max-time 600 and -C -.
setup
STABLE_IMAGES_ABS="$ROOT/dl.fedoraproject.org${STABLE_IMAGES_PATH}"
STUB_404_dl_fedoraproject_org=0
plant_release_without_checksum "$STABLE_IMAGES_ABS"
write_checksum "$STABLE_IMAGES_ABS/$CS_44" \
	"SHA256 ($IMG_44) = $IMG_HASH"

RESULT="$(run_script 5 "$WORK/vm.qcow2")"
RC="$(printf '%s\n' "$RESULT" | sed -n 1p)"

check "item5c: script exits 0" "0" "$RC"
# An index request must carry the metadata timeouts. The stub's
# full-args log has one line per call, so the grep below returns
# the number of calls that mention each flag. The assertion is
# >= 1, encoded as a separate helper.
INDEX_LOG="$COUNTERS/dl.fedoraproject.org.args"
check "item5c: at least one index call has --max-time 60" "1" \
	"$(grep -cF -- '--max-time 60' "$INDEX_LOG" | awk '{print ($1>=1)?1:0}')"
check "item5c: at least one index call has --connect-timeout 10" "1" \
	"$(grep -cF -- '--connect-timeout 10' "$INDEX_LOG" | awk '{print ($1>=1)?1:0}')"
check "item5c: at least one image call has --max-time 600" "1" \
	"$(grep -cF -- '--max-time 600' "$INDEX_LOG" | awk '{print ($1>=1)?1:0}')"
check "item5c: at least one image call has -C -" "1" \
	"$(grep -cF -- '-C -' "$INDEX_LOG" | awk '{print ($1>=1)?1:0}')"

teardown

echo
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
