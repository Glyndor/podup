#!/usr/bin/env bash
#
# The job that cancels the other runs of a failed commit.
#
# It holds `actions: write`, so every case here is a way it could cancel too
# much, fail the job, or run code the pull request wrote:
#
#   - a run of `push`, `merge_group`, `schedule` or `workflow_dispatch` of the same
#     commit is never cancelled, nor is a release;
#   - the run the job belongs to is never cancelled;
#   - a workflow that is not on the list is never cancelled, and none of the
#     workflows that analyse security is on it;
#   - another commit, another branch or another repository's run is never
#     cancelled;
#   - a listing that fails, a cancellation that is refused and a missing input
#     do not fail the job, and cancel nothing they were not asked to;
#   - the launcher runs the BASE commit's copy of the script and never the one in
#     the pull request's tree, and a base without the script runs nothing.
#
# Requires: python3 with PyYAML, jq, git.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
script="$HERE/.github/scripts/ci/cancel-others.sh"
reusable="$HERE/.github/workflows/reusable-cancel-others.yml"
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

SHA=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
OTHER_SHA=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
mkdir "$WORK/bin"
cat >"$WORK/bin/gh" <<'GH'
#!/usr/bin/env bash
# A stand-in for gh: it lists the runs from $RUNS_JSON and records cancellations.
if [ "$1" = api ] && [ "$2" = -X ] && [ "$3" = POST ]; then
	[ "${CANCEL_FAIL:-0}" = 0 ] || exit 1
	echo "$4" >>"$CANCELLED"
	exit 0
fi
if [ "$1" = api ]; then
	[ "${LIST_FAIL:-0}" = 0 ] || exit 1
	echo "$2" >"$LISTED"
	cat "$RUNS_JSON"
	exit 0
fi
exit 2
GH
chmod +x "$WORK/bin/gh"

# run <id> <event> <status> <path> [sha] [branch] [repo]: one run of the listing.
run_json() {
	printf '{"id":%s,"event":"%s","status":"%s","path":"%s","head_sha":"%s","head_branch":"%s","head_repository":{"full_name":"%s"}}' \
		"$1" "$2" "$3" "$4" "${5:-$SHA}" "${6:-feature}" "${7:-Glyndor/podup}"
}
listing() { # <run json>...
	local out='{"workflow_runs":[' first=1 r
	for r in "$@"; do
		[ "$first" = 1 ] || out+=','
		out+="$r"
		first=0
	done
	printf '%s]}' "$out" >"$WORK/runs.json"
}
cancelled_ids() { # the ids cancelled, sorted, one line
	[ -f "$WORK/cancelled.log" ] || { echo none; return; }
	sed -E 's#.*/runs/([0-9]+)/cancel#\1#' "$WORK/cancelled.log" | sort -n | paste -sd, -
}
go() { # env overrides as arguments; prints the exit status
	rm -f "$WORK/cancelled.log" "$WORK/listed.log"
	env PATH="$WORK/bin:$PATH" RUNS_JSON="$WORK/runs.json" CANCELLED="$WORK/cancelled.log" LISTED="$WORK/listed.log" \
		GH_TOKEN=x REPO=Glyndor/podup RUN_ID=100 HEAD_SHA="$SHA" HEAD_BRANCH=feature "$@" \
		bash "$script" >"$WORK/out.log" 2>&1
	echo $?
}

WF=.github/workflows
# The runs of one commit: the failing one (100, itself), several that may go, and
# every kind that must stay.
listing \
	"$(run_json 100 pull_request in_progress $WF/ci.yml)" \
	"$(run_json 101 pull_request in_progress $WF/ci.yml)" \
	"$(run_json 102 pull_request queued $WF/podman-lane.yml)" \
	"$(run_json 103 pull_request in_progress $WF/debian-build.yml)" \
	"$(run_json 104 push in_progress $WF/ci.yml)" \
	"$(run_json 105 merge_group in_progress $WF/ci.yml)" \
	"$(run_json 106 schedule in_progress $WF/podman-lane.yml)" \
	"$(run_json 107 workflow_dispatch in_progress $WF/ci.yml)" \
	"$(run_json 108 push in_progress $WF/release.yml)" \
	"$(run_json 109 pull_request in_progress $WF/release.yml)" \
	"$(run_json 110 pull_request in_progress $WF/audit.yml)" \
	"$(run_json 111 pull_request in_progress $WF/supply-chain.yml)" \
	"$(run_json 112 pull_request in_progress $WF/commit-hygiene.yml)" \
	"$(run_json 113 pull_request in_progress $WF/dependabot-security-watch.yml)" \
	"$(run_json 114 pull_request in_progress $WF/some-new-workflow.yml)" \
	"$(run_json 115 pull_request completed $WF/ci.yml)" \
	"$(run_json 116 pull_request in_progress $WF/ci.yml $OTHER_SHA)" \
	"$(run_json 117 pull_request in_progress $WF/ci.yml $SHA other-branch)" \
	"$(run_json 118 pull_request in_progress $WF/ci.yml $SHA feature someone/podup)" \
	"$(run_json 119 pull_request waiting $WF/lint-shell.yml)" \
	"$(run_json 120 pull_request in_progress "$WF/ci.yml@refs/pull/9/merge")"
check "the job exits 0" 0 "$(go)"
check "it cancels exactly the unfinished pull request runs of this commit and branch on the list" \
	"101,102,103,119,120" "$(cancelled_ids)"
check "the listing asks for this commit and for pull requests only" 1 \
	"$(grep -cF "head_sha=${SHA}&event=pull_request" "$WORK/listed.log")"
for kept in 104:"a push run" 105:"a merge group run" 106:"a scheduled run" 107:"a dispatched run" \
	108:"a release push run" 109:"a release run of a pull request" 110:"the dependency audit" \
	111:"the supply chain check" 112:"the secret scan" 113:"the Dependabot security watch" \
	114:"a workflow nobody listed" 115:"a finished run" 116:"another commit" 117:"another branch" \
	118:"another repository" 100:"its own run"; do
	id="${kept%%:*}"
	check "it never cancels ${kept#*:}" 0 "$(grep -c "/runs/$id/cancel" "$WORK/cancelled.log")"
done

# Failures never fail the job, and a refused cancellation is reported.
check "an unreadable listing exits 0" 0 "$(go LIST_FAIL=1)"
check "an unreadable listing cancels nothing" none "$(cancelled_ids)"
check "an unreadable listing is reported" 1 "$(grep -c 'run list could not be read' "$WORK/out.log")"
check "a refused cancellation exits 0" 0 "$(go CANCEL_FAIL=1)"
check "a refused cancellation is reported" 1 "$(grep -c 'run 101 of .github/workflows/ci.yml could not be cancelled' "$WORK/out.log")"
printf 'not json' >"$WORK/runs.json"
check "a listing that is not JSON exits 0" 0 "$(go)"
check "a listing that is not JSON cancels nothing" none "$(cancelled_ids)"
listing "$(run_json 101 pull_request in_progress $WF/ci.yml)"
check "a missing head commit exits 0" 0 "$(go HEAD_SHA=)"
check "a missing head commit cancels nothing" none "$(cancelled_ids)"
check "a head commit that is not a hash cancels nothing" none "$(go 'HEAD_SHA=a;b' >/dev/null; cancelled_ids)"
check "a missing branch cancels nothing" none "$(go HEAD_BRANCH= >/dev/null; cancelled_ids)"
check "a run id that is not a number cancels nothing" none "$(go 'RUN_ID=1;2' >/dev/null; cancelled_ids)"
check "a missing repository cancels nothing" none "$(go REPO= >/dev/null; cancelled_ids)"
# A branch name with shell and jq metacharacters is data, not code.
listing "$(run_json 101 pull_request in_progress $WF/ci.yml "$SHA" 'x\" or true or \"')"
check "a hostile branch name matches nothing and cancels nothing" none "$(go 'HEAD_BRANCH=feature' >/dev/null; cancelled_ids)"
check "a hostile branch name is compared as a value" 101 "$(go 'HEAD_BRANCH=x" or true or "' >/dev/null; cancelled_ids)"

# No security workflow is on the list, and every file on it exists.
python3 - "$script" >"$WORK/list.txt" <<'PY'
import re
import sys

text = open(sys.argv[1]).read()
block = re.search(r"CANCELLABLE=\((.*?)\n\)", text, re.S).group(1)
print("\n".join(line.strip() for line in block.splitlines() if line.strip()))
PY
check "the list holds ten workflows" 10 "$(grep -c . "$WORK/list.txt")"
check "no workflow that analyses security is on the list" 0 \
	"$(grep -ciE 'codeql|audit|supply|secret|hygiene|dco|dependabot|security|scan|release|publish' "$WORK/list.txt")"
missing=0
while IFS= read -r wf; do [ -f "$HERE/$wf" ] || missing=$((missing + 1)); done <"$WORK/list.txt"
check "every workflow on the list exists" 0 "$missing"
# Sabotage: adding the audit to the list is what the test above must catch.
sed 's#^\t.github/workflows/fuzz.yml#\t.github/workflows/fuzz.yml\n\t.github/workflows/audit.yml#' "$script" >"$WORK/sabotaged.sh"
check "the sabotaged list differs" 1 "$(cmp -s "$script" "$WORK/sabotaged.sh" && echo 0 || echo 1)"
check "an audit on the list would be caught" 1 \
	"$(sed -n '/^CANCELLABLE=(/,/^)/p' "$WORK/sabotaged.sh" | grep -ciE 'audit')"

# The launcher: the step is read from the workflow and run in a repository with a
# base commit and a head commit.
python3 - "$reusable" >"$WORK/launcher.sh" <<'PY'
import sys
import yaml

spec = yaml.safe_load(open(sys.argv[1]))
steps = spec["jobs"]["cancel"]["steps"]
print(steps[1]["run"])
PY
repo="$WORK/repo"
git init -q -b main "$repo"
git -C "$repo" config user.email t@example.invalid
git -C "$repo" config user.name t
mkdir -p "$repo/.github/scripts/ci"
launch() { # <base sha>: run the launcher in the repo at HEAD
	(cd "$repo" && env RUNNER_TEMP="$WORK/tmp" BASE_SHA="$1" GH_TOKEN=x REPO=Glyndor/podup RUN_ID=1 \
		HEAD_SHA="$SHA" HEAD_BRANCH=feature bash "$WORK/launcher.sh" 2>&1)
}
mkdir -p "$WORK/tmp"
# The base has no script; the head adds one that would announce itself.
echo base >"$repo/README.md"
git -C "$repo" add -A && git -C "$repo" commit -q -m base
base_no_script="$(git -C "$repo" rev-parse HEAD)"
printf '#!/usr/bin/env bash\necho EXECUTED-FROM-THE-PULL-REQUEST\n' >"$repo/.github/scripts/ci/cancel-others.sh"
git -C "$repo" add -A && git -C "$repo" commit -q -m head
out="$(launch "$base_no_script")"
check "a base without the script runs nothing" 0 "$(printf '%s\n' "$out" | grep -c EXECUTED)"
check "a base without the script says so" 1 "$(printf '%s\n' "$out" | grep -c 'base commit has no cancel script')"
# The base has its own script; the head replaces it.
git -C "$repo" checkout -q "$base_no_script"
mkdir -p "$repo/.github/scripts/ci"
printf '#!/usr/bin/env bash\necho RAN-THE-BASE-COPY\n' >"$repo/.github/scripts/ci/cancel-others.sh"
git -C "$repo" add -A && git -C "$repo" commit -q -m "base with script"
base_script="$(git -C "$repo" rev-parse HEAD)"
printf '#!/usr/bin/env bash\necho EXECUTED-FROM-THE-PULL-REQUEST\n' >"$repo/.github/scripts/ci/cancel-others.sh"
git -C "$repo" add -A && git -C "$repo" commit -q -m "head replaces it"
out="$(launch "$base_script")"
check "the base commit's copy runs" 1 "$(printf '%s\n' "$out" | grep -c RAN-THE-BASE-COPY)"
check "the pull request's copy does not run" 0 "$(printf '%s\n' "$out" | grep -c EXECUTED)"
check "the launcher does not run a file of the working tree" 0 "$(grep -cE 'bash (\./|\.github|scripts)' "$WORK/launcher.sh")"
# shellcheck disable=SC2016
check "the event values reach the launcher through env, not through the text" 0 "$(grep -c '\${{' "$WORK/launcher.sh")"

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
