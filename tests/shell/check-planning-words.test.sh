#!/usr/bin/env bash
#
# Behaviour tests for `.github/scripts/check-planning-words.sh`.
#
# The script fails when a tracked file names a planning note ("the brief
# calls this out") instead of stating what the code does. Each case plants
# files in a throwaway git repository, runs the real script there, and
# reads its exit status directly rather than through a pipeline, so a
# `head` or `grep` downstream cannot answer for it.
#
# Cases:
#   - a clean tree passes
#   - the word in a tracked comment fails, and the annotation names the
#     file and the line
#   - the match ignores case (`Briefing` fails)
#   - a longer word that merely starts with it (`briefly`) passes: the
#     match is on the whole word
#   - CHANGELOG.md is exempt
#   - an untracked file is not read: the gate is about what ships
#
# Requires: git, bash.
set -u

cd "$(dirname "$0")/../.." || exit 1
script="$PWD/.github/scripts/check-planning-words.sh"

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

res="$(run_case src/a.rs '// adds two numbers')"
check "a clean tree passes" 0 "$(printf '%s\n' "$res" | sed -n 1p)"

res="$(run_case src/a.rs $'fn main() {}\n// the brief calls this out')"
check "the word in a tracked comment fails" 1 "$(printf '%s\n' "$res" | sed -n 1p)"
check "the annotation names the file and the line" \
	"::error file=src/a.rs,line=2::names a planning note; say what the code does or pins instead" \
	"$(printf '%s\n' "$res" | grep -m1 '^::error')"

res="$(run_case docs/x.md 'See the Briefing for why.')"
check "the match ignores case" 1 "$(printf '%s\n' "$res" | sed -n 1p)"

res="$(run_case docs/x.md 'This waits briefly before retrying.')"
check "a longer word that starts with it passes" 0 "$(printf '%s\n' "$res" | sed -n 1p)"

res="$(run_case CHANGELOG.md '- the brief was wrong about the default')"
check "CHANGELOG.md is exempt" 0 "$(printf '%s\n' "$res" | sed -n 1p)"

res="$(run_case src/a.rs '// fine' notes.txt 'the brief')"
check "an untracked file is not read" 0 "$(printf '%s\n' "$res" | sed -n 1p)"

echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
