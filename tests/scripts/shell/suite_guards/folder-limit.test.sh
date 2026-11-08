#!/usr/bin/env bash
# Behaviour tests for .github/scripts/checks/folder-limit.sh on real git indexes.
#
# Ported from authcore's scripts/folder-limit test without the part that runs its
# workflow's base loader: here the script runs as a step of the lint job.
set -euo pipefail
export LC_ALL=C

here="$(cd "$(dirname "$0")" && pwd)"
script="$here/../../../../.github/scripts/checks/folder-limit.sh"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
passed=0
failed=0
exceptions=.github/folder-limit-exceptions.txt

new_repo() {
	local name="$1"
	mkdir -p "$tmp/$name"
	cd "$tmp/$name"
	git init -q
	git config user.email t@example.com
	git config user.name t
	git config commit.gpgsign false
	git config core.hooksPath /dev/null
	unset BASE
}

files() {
	local dir="$1" count="$2" i
	mkdir -p "$dir"
	for ((i = 1; i <= count; i++)); do : >"$dir/file $i"; done
	git add -- "$dir"
}

exception() {
	mkdir -p .github
	printf '%s\t%s\t%s\n' "$1" "$2" 'Fixture with a documented reason.' >"$exceptions"
	git add -- "$exceptions"
}

# Assert exit status AND a diagnostic row, so an unrelated shell failure cannot
# stand in for an intended refusal. Empty NEEDLE only checks the exit status.
expect() {
	local label="$1" want="$2" needle="$3" got=0 output
	shift 3
	if output="$("$@" 2>&1)"; then got=0; else got=$?; fi
	if [ "$got" -eq "$want" ] && { [ -z "$needle" ] || [[ "$output" == *"$needle"* ]]; }; then
		passed=$((passed + 1))
	else
		printf 'FAIL: %s (want status %d, got %d; need %q)\n%s\n' "$label" "$want" "$got" "$needle" "$output" >&2
		failed=$((failed + 1))
	fi
}

new_repo twenty
files leaf 20
expect '20 direct files' 0 'folder-limit: OK' bash "$script"
: >leaf/untracked
expect 'untracked file does not count' 0 'folder-limit: OK' bash "$script"
rm leaf/untracked
files leaf/child 20
expect 'child files are not counted in parent' 0 'folder-limit: OK' bash "$script"
files leaf 21
expect '21 without exception' 1 $'leaf\t21\t20\ttoo many direct files' bash "$script"
exception leaf 22
expect 'exception below ceiling' 0 'folder-limit: OK' bash "$script"
files leaf 22
expect 'exception at ceiling' 0 'folder-limit: OK' bash "$script"
files leaf 23
expect 'exception over ceiling' 1 $'leaf\t23\t22\ttoo many direct files' bash "$script"
git rm -qf -- 'leaf/file 23' 'leaf/file 22' 'leaf/file 21'
expect 'dead exception at 20' 1 $'leaf\t20\t22\tdead exception' bash "$script"
git rm -qf -- 'leaf/file 20'
expect 'dead exception below 20' 1 $'leaf\t19\t22\tdead exception' bash "$script"
git rm -qrf -- leaf
expect 'exception for removed folder' 1 $'leaf\t0\t22\tdead exception' bash "$script"

new_repo exclusions
files .github/workflows 21
expect 'flat workflows' 0 'folder-limit: OK' bash "$script"
files .github/workflows/nested 21
expect 'workflow exclusion applies only to flat folder' 1 $'.github/workflows/nested\t21\t20' bash "$script"
git rm -qrf -- .github/workflows/nested
files testdata 21
expect 'testdata is counted like any other folder' 1 $'testdata\t21\t20' bash "$script"

new_repo spaces
files 'folder with spaces' 20
expect 'spaces in folder and file names at limit' 0 'folder-limit: OK' bash "$script"
files 'folder with spaces' 21
expect 'spaces still count over limit' 1 $'folder\\ with\\ spaces\t21\t20' bash "$script"
exception 'folder with spaces' 21
expect 'exception with spaces' 0 'folder-limit: OK' bash "$script"

new_repo root
files . 21
expect 'root folder' 1 $'.\t21\t20' bash "$script"
exception . 21
expect 'root exception' 0 'folder-limit: OK' bash "$script"

new_repo ordered
files zebra 21
files alpha 22
expect 'offenders sorted by folder' 1 $'alpha\t22\t20\ttoo many direct files\nzebra\t21\t20' bash "$script"

new_repo format
files leaf 21
exception leaf 21
printf 'leaf\t22\tDuplicate\n' >>"$exceptions"
expect 'duplicate exception' 1 'duplicate exception' bash "$script"
printf 'leaf\t21\t\n' >"$exceptions"
expect 'missing reason' 1 'expected exactly three fields and a reason' bash "$script"
printf 'leaf 21 Reason\n' >"$exceptions"
expect 'spaces cannot replace tabs' 1 'expected path<TAB>ceiling<TAB>reason' bash "$script"
exception leaf 020
expect 'reject octal ceiling' 1 'ceiling must be a decimal integer' bash "$script"
exception leaf 20
expect 'exception ceiling must exceed default' 1 'ceiling must be a decimal integer' bash "$script"
exception ./leaf 21
expect 'reject noncanonical exception path' 1 'use a relative folder' bash "$script"
exception leaf/ 21
expect 'reject trailing slash alias' 1 'use a relative folder' bash "$script"
exception leaf 21
printf '# Comment\n\n' >>"$exceptions"
expect 'comments and blank lines' 0 'folder-limit: OK' bash "$script"
printf 'leaf\t21\tReason without final newline' >"$exceptions"
expect 'last line without newline' 0 'folder-limit: OK' bash "$script"

new_repo ratchet
files leaf 21
exception leaf 22
git commit -qm base
base="$(git rev-parse HEAD)"
expect 'unchanged ceiling' 0 'folder-limit: OK' env BASE="$base" bash "$script"
exception leaf 23
expect 'ceiling cannot increase' 1 $'leaf\t21\t23\tceiling increased from 22' env BASE="$base" bash "$script"
exception leaf 21
expect 'ceiling can decrease' 0 'folder-limit: OK' env BASE="$base" bash "$script"
expect 'unreadable base is an error' 128 '' env BASE=deadbeef bash "$script"

# Sabotage: the same 21-file folder under a copy whose default ceiling is raised.
# The refusals above are not vacuous because that copy lets the folder through.
# shellcheck disable=SC2016  # the pattern is text to match, not an expansion
sed 's/cap="${ceilings\["$dir"\]:-20}"/cap="${ceilings["$dir"]:-1000}"/' "$script" >"$tmp/sabotaged.sh"
if cmp -s "$script" "$tmp/sabotaged.sh"; then
	printf 'FAIL: the sabotage did not change the script\n' >&2
	failed=$((failed + 1))
else
	new_repo sabotage
	files leaf 21
	expect 'real script refuses 21 direct files' 1 $'leaf\t21\t20' bash "$script"
	expect 'sabotaged ceiling lets the same folder through' 0 'folder-limit: OK' bash "$tmp/sabotaged.sh"
fi

printf 'passed %d, failed %d\n' "$passed" "$failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$passed" "$failed"
if [ "$failed" -ne 0 ]; then exit 1; fi
