#!/usr/bin/env bash
# Behaviour tests for .github/workflows/podman-watch.yml.
#
# The step ships three external calls -- `gh api .../releases/latest`,
# `curl ... repology.org ...`, and `gh issue create` -- and a SemVer guard
# that rejects pre-release tags before any of them. A test that mocks the
# workflow itself stops testing the workflow; the way reusable-schedule-
# freshness.test.sh does it is to extract the step's `run:` body, run it
# with `bash`, and stand up fakes on PATH for the binaries it calls.
#
# The real `gh` is never permitted to run here: this test does not set
# GH_TOKEN to a usable value (the stub ignores it anyway), and the real
# binary lives at /home/jaroc/.local/bin/gh, which a stray run would
# reach. The harness prepends its own bin to PATH and a safety check at
# the top of the run asserts that `command -v gh` resolves to the fake
# one; the run aborts if it does not.
#
# Cases are written so the assertions are read off the logs, never off
# the workflow file's source. That way a refactor that renames a step or
# rewrites a comment does not silently break a check that was naming a
# string the user never reads.
#
# Requires: python3, GNU coreutils.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

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

# Pull one step's `run:` body out of a workflow, dedented, so it can be run.
# Same helper as reusable-schedule-freshness.test.sh.
step_script() { # $1=workflow path  $2=step name substring
	python3 - "$1" "$2" <<'PY'
import sys
lines = open(sys.argv[1]).read().splitlines()
start = next(i for i, l in enumerate(lines) if "name: " + sys.argv[2] in l)
run = next(i for i, l in enumerate(lines) if i > start and l.strip() == "run: |")
body = []
for line in lines[run + 1:]:
    if not line.strip():
        body.append("")
        continue
    if not line.startswith(" " * 10):
        break
    body.append(line[10:])
print("\n".join(body))
PY
}

# Fake `gh`. Three shapes are accepted; anything else exits 99 with a
# message on stderr naming what it saw, so a future call we did not
# anticipate surfaces immediately instead of silently returning whatever
# was last in the responses file.
#
#   gh api repos/containers/podman/releases/latest ...   -> $FAKE_TAG
#   gh issue list ...                                   -> empty
#   gh issue create ...                                 -> logged + URL
#
# `gh issue create` writes its argv (joined with spaces, NUL-terminated)
# to $GH_LOG so the test can read it back.
write_stubs() {
	mkdir -p "$WORK/bin"
	cat > "$WORK/bin/gh" <<'STUB'
#!/usr/bin/env bash
set -u
LOG="${GH_LOG:?gh log path required}"
URL="${GH_FAKE_URL:?gh fake URL required}"
TAG="${FAKE_TAG:?fake tag required}"
joined="$*"
printf '%s\0' "$joined" >> "$LOG"
case "$joined" in
	"api repos/containers/podman/releases/latest"*)
		printf '%s\n' "$TAG"
		exit 0
		;;
	"issue list"*)
		exit 0
		;;
	"issue create"*)
		printf '%s\n' "$URL"
		exit 0
		;;
	*)
		echo "fake-gh: unexpected invocation: $joined" >&2
		exit 99
		;;
esac
STUB
	chmod +x "$WORK/bin/gh"
	cat > "$WORK/bin/curl" <<'STUB'
#!/usr/bin/env bash
set -u
LOG="${CURL_LOG:?curl log path required}"
RESP="${CURL_RESPONSES:?curl responses file required}"
printf '%s\0' "$*" >> "$LOG"
# Each call appends one NUL-terminated record, so the count of records
# equals the next call's index into the line-based responses file.
idx=$(awk -v RS='\0' 'END {print NR+0}' "$LOG")
line=$(awk -v n="$idx" 'NR==n {print; exit}' "$RESP")
exit_code="${line%%	*}"
body="${line#*	}"
if [ "$exit_code" != "0" ]; then
	# Real curl prints nothing and exits non-zero on transport failure.
	exit "$exit_code"
fi
printf '%s' "$body"
exit 0
STUB
	chmod +x "$WORK/bin/curl"
	cat > "$WORK/bin/sleep" <<'STUB'
#!/usr/bin/env bash
set -u
printf '%s\0' "$*" >> "${SLEEP_LOG:?sleep log path required}"
STUB
	chmod +x "$WORK/bin/sleep"
}

WORKFLOW="$HERE/.github/workflows/podman-watch.yml"
step_script "$WORKFLOW" "Detect newer packaged Podman" > "$WORK/step.sh"

# Build the per-case environment. Each case gets its own baseline file in
# $WORK/repo/.github/podman-baseline and its own log directory so logs from
# a previous case never bleed into the next.
write_stubs

# --- safety: the fake gh must be the gh on PATH inside the step ----------
#
# The harness's real `gh` is /home/jaroc/.local/bin/gh and has a logged-in
# session. Running the step with that one on PATH would either talk to
# github.com (silent, irreversible) or, worse, take a branch we did not
# consent to. Run the step once with the baseline equal to the fake tag
# so the "No newer Podman" early-exit fires before any external call, then
# assert that the only `gh` it could have found is our stub.
mkdir -p "$WORK/safety/.github"
printf '%s\n' "6.1.2" > "$WORK/safety/.github/podman-baseline"
out="$(cd "$WORK/safety" && PATH="$WORK/bin:$PATH" \
	GH_LOG="$WORK/safety/gh.log" \
	CURL_LOG="$WORK/safety/curl.log" \
	SLEEP_LOG="$WORK/safety/sleep.log" \
	CURL_RESPONSES="$WORK/safety/empty.resp" \
	FAKE_TAG="v6.1.2" \
	GH_FAKE_URL="https://example.invalid/issues/0" \
	GH_TOKEN=dummy \
	bash "$WORK/step.sh" 2>&1)"
rc=$?
check "safety: the harness can run the step end-to-end" "0" "$rc"
check "safety: command -v gh in the step's PATH resolves to the fake" \
	"$WORK/bin/gh" \
	"$(cd "$WORK/safety" && PATH="$WORK/bin:$PATH" command -v gh)"
# A missed fake would have advanced the curl log past zero or made a real
# `gh issue list` call. Belt and braces.
check "safety: the step called curl zero times before the baseline check" \
	"0" "$(grep -caz . "$WORK/safety/curl.log" 2>/dev/null || echo 0)"

# Helper: build a fresh per-case repo dir, write its baseline, run the
# step, and emit nothing on stdout -- the caller reads the captured out/rc
# from the function output. Stdin via the here-string would be ambiguous
# with the function's own stdin, so the script writes to a file and the
# caller `cat`s it.
run_case() { # <baseline> <fake_tag> <responses_file> <gh_log> <curl_log> <sleep_log> <fake_url>
	local baseline="$1" tag="$2" resp="$3"
	local gh_log="$4" curl_log="$5" sleep_log="$6" fake_url="$7"
	local repo="$WORK/case"
	rm -rf "$repo"
	mkdir -p "$repo/.github"
	printf '%s\n' "$baseline" > "$repo/.github/podman-baseline"
	: > "$gh_log"; : > "$curl_log"; : > "$sleep_log"
	( cd "$repo" && PATH="$WORK/bin:$PATH" \
		GH_LOG="$gh_log" CURL_LOG="$curl_log" SLEEP_LOG="$sleep_log" \
		CURL_RESPONSES="$resp" \
		FAKE_TAG="$tag" GH_FAKE_URL="$fake_url" \
		GH_TOKEN=dummy \
		bash "$WORK/step.sh" ) 2>&1
}

# NUL-separated log records. Each invocation appends a record followed
# by a NUL byte, so the count of records is the count of NUL bytes.
count_calls() { awk -v RS='\0' 'END {print NR+0}' "$1" 2>/dev/null; }

gh_called_with() { # <log> <substring>
	awk -v RS='\0' -v needle="$2" '$0 ~ needle {found=1; exit} END {print (found ? 1 : 0)}' "$1"
}

gh_issue_create_calls() { # <log>
	awk -v RS='\0' 'index($0, "issue create") == 1 {c++} END {print (c+0)}' "$1"
}

gh_issue_create_args_have() { # <log> <substring>
	awk -v RS='\0' -v needle="$2" 'index($0, "issue create") == 1 && index($0, needle) {found=1; exit} END {print (found ? 1 : 0)}' "$1"
}

FAKE_URL="https://example.invalid/issues/42"

# --- W1: baseline equals tag => exit 0, no curl --------------------------
: > "$WORK/w1.empty.resp"
out="$(run_case "6.1.2" "v6.1.2" "$WORK/w1.empty.resp" \
	"$WORK/w1.gh.log" "$WORK/w1.curl.log" "$WORK/w1.sleep.log" "$FAKE_URL")"
rc=$?
check "W1: baseline equals tag exits 0" "0" "$rc"
check "W1: output says no newer Podman than the validated baseline" "1" \
	"$(printf '%s' "$out" | grep -q 'No newer Podman than the validated baseline (6.1.2)' && echo 1 || echo 0)"
check "W1: curl was called 0 times" "0" "$(count_calls "$WORK/w1.curl.log")"
check "W1: gh issue create was never called" "0" \
	"$(gh_issue_create_calls "$WORK/w1.gh.log")"

# --- W2: curl exits 7 three times => exit 1 with the named error --------
printf '7\t\n7\t\n7\t\n' > "$WORK/w2.curl.resp"
out="$(run_case "6.1.1" "v6.1.2" "$WORK/w2.curl.resp" \
	"$WORK/w2.gh.log" "$WORK/w2.curl.log" "$WORK/w2.sleep.log" "$FAKE_URL")"
rc=$?
check "W2: three curl failures exit 1" "1" "$rc"
check "W2: curl was called exactly 3 times" "3" "$(count_calls "$WORK/w2.curl.log")"
check "W2: sleep was called exactly 2 times" "2" "$(count_calls "$WORK/w2.sleep.log")"
check "W2: each sleep argument was 20" "20 20" \
	"$(tr '\0' '\n' < "$WORK/w2.sleep.log" | xargs)"
check "W2: output names repology as the failed service" "1" \
	"$(printf '%s' "$out" | grep -q 'repology.org did not answer after 3 attempts (curl exit 7)' && echo 1 || echo 0)"
check "W2: output does not include a Python Traceback" "0" \
	"$(printf '%s' "$out" | grep -c 'Traceback')"
check "W2: output does not include a JSONDecodeError" "0" \
	"$(printf '%s' "$out" | grep -c 'JSONDecodeError')"
check "W2: gh issue create was never called" "0" \
	"$(gh_issue_create_calls "$WORK/w2.gh.log")"

# --- W3: one curl failure, then a JSON list => issue opened --------------
printf '7\t\n0\t[{"repo":"fedora_rawhide","version":"6.1.2"},{"repo":"debian_13","version":"5.4.2"}]\n' > "$WORK/w3.curl.resp"
out="$(run_case "6.1.1" "v6.1.2" "$WORK/w3.curl.resp" \
	"$WORK/w3.gh.log" "$WORK/w3.curl.log" "$WORK/w3.sleep.log" "$FAKE_URL")"
rc=$?
check "W3: one failure then a JSON list exits 0" "0" "$rc"
check "W3: curl was called exactly 2 times" "2" "$(count_calls "$WORK/w3.curl.log")"
check "W3: sleep was called exactly once" "1" "$(count_calls "$WORK/w3.sleep.log")"
check "W3: that one sleep was 20 seconds" "20" \
	"$(tr '\0' '\n' < "$WORK/w3.sleep.log" | head -n 1)"
check "W3: gh issue create was called exactly once" "1" \
	"$(gh_issue_create_calls "$WORK/w3.gh.log")"
check "W3: the issue title names the version that is now packaged" "1" \
	"$(gh_issue_create_args_have "$WORK/w3.gh.log" "Validate podup on Podman 6.1.2 (now packaged)")"
check "W3: the issue body names the distro that ships 6.1.2 (fedora_rawhide)" "1" \
	"$(gh_issue_create_args_have "$WORK/w3.gh.log" "fedora_rawhide")"
check "W3: the issue body does NOT name a distro that ships only 5.4.2" "0" \
	"$(gh_issue_create_args_have "$WORK/w3.gh.log" "debian_13")"

# --- W4: curl exits 0 with HTML => JSON parse fails, named error ---------
printf '0\t<html>blocked</html>\n' > "$WORK/w4.curl.resp"
out="$(run_case "6.1.1" "v6.1.2" "$WORK/w4.curl.resp" \
	"$WORK/w4.gh.log" "$WORK/w4.curl.log" "$WORK/w4.sleep.log" "$FAKE_URL")"
rc=$?
check "W4: HTML body exits 1" "1" "$rc"
check "W4: output names the JSON parse failure" "1" \
	"$(printf '%s' "$out" | grep -q 'not with the JSON package list' && echo 1 || echo 0)"
check "W4: output does not include a Python Traceback" "0" \
	"$(printf '%s' "$out" | grep -c 'Traceback')"
check "W4: gh issue create was never called" "0" \
	"$(gh_issue_create_calls "$WORK/w4.gh.log")"

# --- W5: JSON list where no package has 6.1.2 => exit 0, no issue --------
printf '0\t[{"repo":"fedora_rawhide","version":"6.1.1"},{"repo":"debian_13","version":"5.4.2"}]\n' > "$WORK/w5.curl.resp"
out="$(run_case "6.1.1" "v6.1.2" "$WORK/w5.curl.resp" \
	"$WORK/w5.gh.log" "$WORK/w5.curl.log" "$WORK/w5.sleep.log" "$FAKE_URL")"
rc=$?
check "W5: a JSON list with no 6.1.2 exits 0" "0" "$rc"
check "W5: output says not packaged in any distro yet" "1" \
	"$(printf '%s' "$out" | grep -q 'not packaged in any distro yet' && echo 1 || echo 0)"
check "W5: gh issue create was never called" "0" \
	"$(gh_issue_create_calls "$WORK/w5.gh.log")"

# --- W6: pre-release tag fails the SemVer guard before any curl ----------
: > "$WORK/w6.empty.resp"
out="$(run_case "6.1.1" "v6.1.2-rc1" "$WORK/w6.empty.resp" \
	"$WORK/w6.gh.log" "$WORK/w6.curl.log" "$WORK/w6.sleep.log" "$FAKE_URL")"
rc=$?
check "W6: a pre-release tag exits 1" "1" "$rc"
check "W6: the error names the rejected tag" "1" \
	"$(printf '%s' "$out" | grep -q 'unexpected Podman release tag' && echo 1 || echo 0)"
check "W6: curl was called 0 times (guard fires first)" "0" \
	"$(count_calls "$WORK/w6.curl.log")"

echo
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
