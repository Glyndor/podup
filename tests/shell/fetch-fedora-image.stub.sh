#!/usr/bin/env bash
#
# Shared plumbing for the fetch-fedora-image tests.
#
# This file is sourced (not executed). It defines the stub curl that
# stands in for the real binary, the pass/fail counters and check
# helper, the per-case setup/teardown, the fixture names, the run
# helper, the per-path 404 limit helper, and the log-asset helpers
# both fetch-fedora-image.test.sh and fetch-fedora-image-records.test.sh
# need. The two test files own the per-case planters and the
# assertions; this file is what they share.
#
# Sourced files are not invoked, so the executable bit is not required;
# the gate in reusable-shell-ci.yml reads the mode git records, and
# this file is 0644 on purpose.

set -u

cd "$(dirname "${BASH_SOURCE[0]}")/../.." || exit 1
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
#   4. appends the FULL argument list to $STUB_COUNTERS_DIR/<host>.args
#      so a test can grep for the flags the script should have set
#      (--max-redirs 0 on CHECKSUM, --max-time on metadata, etc).
#   5. counts how many times each (host, path) pair has been called,
#      under $STUB_COUNTERS_DIR/<host>.<sanitized-path>, where the
#      sanitizer turns `/` and `.` into `_` so the result is a single
#      filename,
#   6. decides the 404 limit for this call:
#      - if $STUB_LIMITS_DIR/<host>/<path> exists, its content is the
#        per-(host,path) limit; the per-(host,path) counter is compared
#        against it,
#      - otherwise STUB_404_<host> is the per-host limit; the per-host
#        counter is compared against it.
#   7. if the call count for the chosen scope is <= that limit, exits
#      with curl's HTTP error code (22) so the caller's `curl -f` is
#      honoured,
#   8. otherwise copies $STUB_ROOT/<host>/<path> to stdout, or to
#      `-o`'d file when the script used -o. A URL ending in `/` is
#      treated as a directory listing (Apache serves index.html for
#      that path), so the stub mirrors it as well.
#   9. if a per-(host, path) marker `$LIMITS/<host>/<path>.exit_after`
#      exists, the stub copies the body AND exits with the code in
#      the marker. This is what makes the item-1 "partial CHECKSUM
#      body" test bite: the marker forces a curl exit-18 case while
#      still streaming the CHECKSUM body to the script's command
#      substitution, the way a real hung connection would.
#
# The stub is POSIX sh because it runs as /bin/sh from the caller's
# PATH. The heredoc is quoted ('STUB') on purpose so the $1, $STUB_*
# etc. are written verbatim to the file -- the stub expands them at
# run time from its own environment.
write_stub() { # <path>
	cat > "$1" <<'STUB'
#!/bin/sh
set -eu
COUNTERS="$STUB_COUNTERS_DIR"
ROOT="$STUB_ROOT"
LIMITS="$STUB_LIMITS_DIR"
out=""
url=""
# Capture the full argument list BEFORE the parsing loop below
# shifts them; after the loop $@ is empty. The log line below is
# what the records test greps for --max-redirs 0, --max-time, -C -
# etc., so it has to record the args the way the caller passed
# them, not the way the parser left them.
full_args="$*"
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
# Per-call full argument list, one call per line, in arrival order.
# Tests grep this to assert the flags the script set on this kind of
# call (--max-redirs 0 on CHECKSUM, --max-time on metadata, -C - on
# the image).
printf '%s\n' "$full_args" >> "$COUNTERS/$host.args"
# Per-(host, path) counter. `/` and `.` collapse to `_` so the
# result fits a single filename; the original path is still on the
# per-host log above for human inspection.
sanitized=$(printf '%s' "$path" | tr '/.' '__')
path_counter="$COUNTERS/${host}.${sanitized}"
if [ ! -f "$path_counter" ]; then
	echo 0 > "$path_counter"
fi
path_count=$(cat "$path_counter")
path_count=$((path_count + 1))
echo "$path_count" > "$path_counter"
# Resolve the 404 limit. A per-(host, path) limit in $LIMITS
# overrides the per-host STUB_404_<host> for that path.
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
# Per-(host, path) "print body and exit with code" override. Used by
# the item-1 test: the CHECKSUM request returns curl exit 18 (partial
# file) while the body has already been streamed, so a buggy command
# substitution that ignores the exit code would still capture the
# body. The marker lives at $LIMITS/<host>/<path>.exit_after; its
# content is the exit code.
exit_after_file="$LIMITS/$host/$path.exit_after"
if [ -n "$LIMITS" ] && [ -f "$exit_after_file" ]; then
	file="$ROOT/$host/$path"
	case "$path" in
		*/) file="$file"index.html ;;
	esac
	if [ -f "$file" ]; then
		if [ -n "$out" ]; then
			cat "$file" > "$out"
		else
			cat "$file"
		fi
	fi
	exit "$(cat "$exit_after_file")"
fi
file="$ROOT/$host/$path"
# A URL ending in `/` is a directory listing. Apache serves
# index.html for that path; the stub mirrors that so the script's
# directory-fetch call lands on the planted file.
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
# Build the fixture tree (stub curl + per-host content roots) and
# reset the per-host 404 counters and limits. Each case owns a fresh
# $WORK so cross-case state cannot leak. The caller sets env to
# inject per-host 404 limits (STUB_404_<host>) and per-path 404 limits
# (a file under $LIMITS/) before invoking the script.
setup() {
	WORK="$(mktemp -d)"
	STUB_DIR="$WORK/stub"
	ROOT="$WORK/roots"
	COUNTERS="$WORK/counters"
	LIMITS="$WORK/limits"
	mkdir -p "$STUB_DIR" "$ROOT" "$COUNTERS" "$LIMITS"

	write_stub "$STUB_DIR/curl"
	chmod +x "$STUB_DIR/curl"

	# Per-case 404 limits are exported before run_script so the stub
	# sees them. shellcheck cannot see across processes, so disable
	# the unused warning at the assignment site.
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
# A real Fedora 44 image name from the live index on 2026-10-02, with
# the corresponding CHECKSUM filename the index carries next to it.
# shellcheck disable=SC2034 # the consumer (the test file) uses them
IMG_44=Fedora-Cloud-Base-Generic-44-1.7.x86_64.qcow2
# shellcheck disable=SC2034 # the consumer (the test file) uses them
CS_44=Fedora-Cloud-44-1.7-x86_64-CHECKSUM

# Rawhide image name and the actual CHECKSUM filename rawhide uses.
# The CHECKSUM filename embeds the compose and the word `images`, so
# it does not match the stable path's pattern; the script does not
# derive it from the image, it parses both out of the index side by
# side.
# shellcheck disable=SC2034 # the consumer (the test file) uses them
IMG_RAWHIDE=Fedora-Cloud-Base-Generic-Rawhide-20261002.n.0.x86_64.qcow2
# shellcheck disable=SC2034 # the consumer (the test file) uses them
CS_RAWHIDE=Fedora-Cloud-images-Rawhide-x86_64-20261002.n.0-CHECKSUM

# Build a synthetic image body and the CHECKSUM line for it. The stub
# serves the body to curl, the script hashes the body and compares
# against what the stub's CHECKSUM claims. This keeps every
# assertion inside the test self-consistent and avoids depending on
# the real Fedora image bytes (which are ~500 MB and out of scope for
# a unit test).
IMG_BODY="$(printf '%.0s;' $(seq 1 2048))"
# shellcheck disable=SC2034 # the consumer (the test file) uses them
IMG_HASH="$(printf '%s' "$IMG_BODY" | sha256sum | awk '{print $1}')"

# --- common run helper --------------------------------------------------------
#
# Run the script with the stub on PATH. Echoes a delimiter-separated
# record: <rc> on its own line, then stdout up to `---`, then stderr
# up to `---`. The caller slices what it needs with `sed`/`awk`.
# Extra env vars (FETCH_DEADLINE and friends) must be `export`ed in
# the caller before this runs, so the subshell inherits them.
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

# Drop a per-(host, path) 404 limit file the stub consumes to decide
# whether THIS call is the first N of a path the host is supposed to
# answer 404 for. The path is the part after the host; the file lives
# at $LIMITS/<host>/<path> with content = count.
plant_path_limit() { # <host> <path> <count>
	local host="$1"
	local path="$2"
	local count="$3"
	mkdir -p "$LIMITS/$host/$(dirname -- "$path")"
	printf '%s' "$count" > "$LIMITS/$host/$path"
}

# Plant a CHECKSUM-body-and-exit-18 marker at
# $LIMITS/<host>/<path>.exit_after. The stub, on each call to that
# path, prints the body the test planted under $ROOT and exits with
# the code in the marker. Used by the item-1 "partial CHECKSUM body"
# test: the stub simulates curl exiting 18 (partial file) while the
# body was streamed, the way a real hung connection would.
plant_exit_after() { # <host> <path> <exit-code>
	local host="$1"
	local path="$2"
	local code="$3"
	mkdir -p "$LIMITS/$host/$(dirname -- "$path")"
	printf '%s' "$code" > "$LIMITS/$host/$path.exit_after"
}

# Stable path the script asks for; the CHECKSUM path is the file
# name appended on top of it. Used by the per-path 404 limit helper
# to plant a limit on the CHECKSUM request only.
# shellcheck disable=SC2034
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

# Assertion: the per-host FULL-ARGV log for `host` has at least one
# call whose args include the given flag. Used for the redirect and
# timeout flag checks in the records test.
assert_args_contain() { # <description> <host> <flag>
	local desc="$1"
	local host="$2"
	local flag="$3"
	local log="$COUNTERS/$host.args"
	if [ -f "$log" ] && grep -qF -- "$flag" "$log"; then
		echo "ok    $desc"
		pass=$((pass + 1))
	else
		echo "FAIL  $desc"
		echo "        $log did not contain $flag"
		fail=$((fail + 1))
	fi
}

# Assertion: the per-host FULL-ARGV log for `host` has zero calls
# whose args include the given flag. Used to assert that the index
# and image fetches do NOT carry --max-redirs 0.
assert_args_omit() { # <description> <host> <flag>
	local desc="$1"
	local host="$2"
	local flag="$3"
	local log="$COUNTERS/$host.args"
	if [ ! -f "$log" ] || ! grep -qF -- "$flag" "$log"; then
		echo "ok    $desc"
		pass=$((pass + 1))
	else
		echo "FAIL  $desc"
		echo "        $log contained $flag in:"
		grep -F -- "$flag" "$log" | sed 's/^/          /'
		fail=$((fail + 1))
	fi
}

# Count how many lines in the per-host FULL-ARGV log contain the
# given substring. Used to pin the number of CHECKSUM requests
# (TRIES) and to assert at-most-one for deadline-stops.
count_args_matching() { # <host> <substring>
	local host="$1"
	local needle="$2"
	local log="$COUNTERS/$host.args"
	if [ ! -f "$log" ]; then
		echo 0
		return
	fi
	grep -cF -- "$needle" "$log" || true
}
