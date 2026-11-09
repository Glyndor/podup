#!/usr/bin/env bash
#
# Behaviour tests for `.github/scripts/checks/check-private-document-references.sh`.
#
# The script fails when a tracked file cites a document tree this repository
# does not have. It stands in for the Rust test `no_private_document_references`
# when the classifier lets a pull request skip the Rust lanes, so it has to
# catch the kinds of file the classifier lets skip: a shell test, a licence, a
# template. Each case plants files in a throwaway git repository, runs the real
# script there and reads its exit status directly.
#
# Cases:
#   - a clean tree passes
#   - either needle in a tracked shell test, licence or template fails, and the
#     annotation names the file and the line
#   - Cargo.lock is not read
#   - an untracked file is not read: the gate is about what ships
#
# Requires: git, bash.
set -u

cd "$(dirname "$0")/../../../.." || exit 1
script="$PWD/.github/scripts/checks/check-private-document-references.sh"

# The needles are assembled at run time so this file does not trip the gate it
# tests.
tree="ai-""context"
dir_needle="standards""/"

pass=0; fail=0

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

# A fresh repository per case, with one tracked file holding `content`.
# Prints the script's exit status on the first line and its output after.
run_case() { # <path> <content> [untracked-path untracked-content]
	local dir rc
	dir="$(mktemp -d)"
	git -C "$dir" init -q
	mkdir -p "$dir/$(dirname "$1")"
	printf '%s\n' "$2" > "$dir/$1"
	git -C "$dir" add -- "$1"
	if [ $# -ge 4 ]; then
		printf '%s\n' "$4" > "$dir/$3"
	fi
	out="$(cd "$dir" && bash "$script" 2>&1)"
	rc=$?
	rm -rf "$dir"
	printf '%s\n%s\n' "$rc" "$out"
}

rc_of() { printf '%s\n' "$1" | sed -n 1p; }

res="$(run_case src/a.rs '// adds two numbers')"
check "a clean tree passes" 0 "$(rc_of "$res")"

res="$(run_case tests/scripts/shell/x.test.sh "$(printf '#!/bin/sh\n# see %s for why' "$tree")")"
check "the tree name in a shell test fails" 1 "$(rc_of "$res")"
check "the annotation names the file and the line" \
	"::error file=tests/scripts/shell/x.test.sh,line=2::cites a document tree that is not in this repository; say what the code does or pins instead" \
	"$(printf '%s\n' "$res" | grep -m1 '^::error')"

res="$(run_case tests/scripts/helper.ps1 "# the rules are in ${dir_needle}ci")"
check "the directory needle in a PowerShell test fails" 1 "$(rc_of "$res")"

res="$(run_case LICENSE "see $tree")"
check "the licence is read" 1 "$(rc_of "$res")"

res="$(run_case .github/ISSUE_TEMPLATE/bug_report.md "see $tree")"
check "an issue template is read" 1 "$(rc_of "$res")"

res="$(run_case Cargo.lock "name = \"$tree\"")"
check "Cargo.lock is not read" 0 "$(rc_of "$res")"

res="$(run_case src/a.rs '// fine' notes.txt "see $tree")"
check "an untracked file is not read" 0 "$(rc_of "$res")"

echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
