#!/usr/bin/env bash
#
# Behaviour tests for `.github/scripts/checks/check-banned-vocabulary.sh`.
#
# Each case plants files in a throwaway git repository, runs the real script
# there and reads its exit status directly, so a pipeline downstream cannot
# answer for it. The vetoed words are assembled at run time so this file does
# not trip the gate it tests: writing them literally would need an exemption
# for this path, and an exempt path is where the next occurrence goes unseen.
#
# Cases:
#   - a clean tree passes
#   - every vetoed word fails, in any case, and the annotation names the file
#     and the line
#   - the openers fail with a capital and pass in the middle of a sentence
#   - the phrases fail, and the superlative one fails only when the word after it
#     is not a hyphen (so best-effort wording passes)
#   - a longer word that merely contains one passes
#   - CHANGELOG.md and untracked files are not read
#   - an exception excuses a use by the text on its line, survives the line
#     moving, does not excuse another file, and fails when it matches nothing
#   - sabotage: with the word list emptied the same planted file passes, which
#     is what the failing cases above would then have reported
#
# Requires: git, bash.
set -u

cd "$(dirname "$0")/../../../.." || exit 1
script="$PWD/.github/scripts/checks/check-banned-vocabulary.sh"

pass=0
fail=0
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

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

# A repository with the script's own directory layout and one tracked file.
n=0
new_repo() {
	n=$((n + 1))
	repo="$tmp/r$n"
	mkdir -p "$repo/.github/scripts/checks"
	cp "${1:-$script}" "$repo/.github/scripts/checks/check-banned-vocabulary.sh"
	(
		cd "$repo" || exit 1
		git init -q
		git config user.email t@example.com
		git config user.name t
		git config commit.gpgsign false
	)
}
run() { # prints the exit status; the output goes to $tmp/out
	(cd "$repo" && bash .github/scripts/checks/check-banned-vocabulary.sh >"$tmp/out" 2>&1)
	echo $?
}
track() { # <path> <content>
	mkdir -p "$repo/$(dirname "$1")"
	printf '%s\n' "$2" >"$repo/$1"
	(cd "$repo" && git add -- "$1")
}

# The vocabulary, split so no literal appears in this file.
words=("de""lve" "lever""age" "lever""aged" "seam""less" "seam""lessly" "rob""ust" "rob""ustness" "compre""hensive" "cutting""-edge" "boa""sts" "blazing"" fast" "revolution""ary")
openers=("Addition""ally" "Further""more" "More""over")
phrases=("In conclu""sion" "It's worth no""ting" "the be""st guess")

new_repo
track src/lib.rs "// a plain sentence about the cache"
check "a clean tree passes" 0 "$(run)"

for w in "${words[@]}"; do
	new_repo
	track src/a.rs "fn f() {}
// this makes it $w for everyone"
	check "the word '$w' fails" 1 "$(run)"
done
check "the annotation names the file and the line" "yes" \
	"$(grep -q '^::error file=src/a.rs,line=2::' "$tmp/out" && echo yes || echo no)"

new_repo
track src/a.rs "// this makes it $(printf '%s' "${words[5]}" | tr '[:lower:]' '[:upper:]') for everyone"
check "the match ignores case" 1 "$(run)"

for o in "${openers[@]}"; do
	new_repo
	track docs/a.md "$o, the cache is cold."
	check "the opener '$o' fails with a capital" 1 "$(run)"
	new_repo
	track docs/a.md "the file is $(printf '%s' "$o" | tr '[:upper:]' '[:lower:]') stashed on disk"
	check "the opener '$o' in lower case in a sentence passes" 0 "$(run)"
done

for p in "${phrases[@]}"; do
	new_repo
	track docs/a.md "x $p y"
	check "the phrase '$p' fails" 1 "$(run)"
done
new_repo
track docs/a.md "a be""st-effort attempt, the be""st-effort kind"
check "'the best-effort' is not the phrase" 0 "$(run)"

new_repo
track README.md "launch $(printf '\xf0\x9f\x9a\x80') now"
check "the rocket emoji fails" 1 "$(run)"

new_repo
track docs/a.md "a ${words[5]}ly written sentence and robustly nothing"
# 'robustly' contains the word but is not the word; the first token is the word plus ly.
check "a longer word containing a vetoed one passes" 0 "$(run)"

new_repo
track CHANGELOG.md "- made it ${words[3]}"
check "CHANGELOG.md is not read" 0 "$(run)"

new_repo
mkdir -p "$repo/src"
printf '// %s\n' "${words[3]}" >"$repo/src/untracked.rs"
check "an untracked file is not read" 0 "$(run)"

# Exceptions.
new_repo
track src/a.rs "// it stays ${words[5]} to the code around it"
printf '# comment\nsrc/a.rs\tstays %s to the code\tthe sentence describes the AST approach\n' "${words[5]}" >"$repo/.github/vocabulary-exceptions.txt"
(cd "$repo" && git add .github/vocabulary-exceptions.txt)
check "an exception excuses the use it names" 0 "$(run)"
track src/a.rs "// line moved down
// it stays ${words[5]} to the code around it"
check "the exception survives the line moving" 0 "$(run)"
track src/b.rs "// it stays ${words[5]} to the code around it"
check "the exception does not excuse another file" 1 "$(run)"
(cd "$repo" && git rm -qf src/b.rs)
track src/a.rs "// nothing vetoed here"
check "an exception that matches nothing fails" 1 "$(run)"
check "and says so" "yes" "$(grep -q 'matches nothing' "$tmp/out" && echo yes || echo no)"
printf 'src/a.rs\tsome text\t\n' >"$repo/.github/vocabulary-exceptions.txt"
check "an exception without a reason fails" 1 "$(run)"

# Sabotage: the same planted file under a script whose word list is emptied.
# If this passed the real script's failing cases for nothing, the cases above
# would be vacuous; they are not, because the sabotaged copy lets the file through.
sed "s/^WORDS='.*'/WORDS='nothing-at-all'/" "$script" >"$tmp/sabotaged.sh"
if cmp -s "$script" "$tmp/sabotaged.sh"; then
	echo "FAIL  the sabotage did not change the script"
	fail=$((fail + 1))
else
	new_repo "$tmp/sabotaged.sh"
	track src/a.rs "// this makes it ${words[3]} for everyone"
	check "with the word list emptied the planted file is let through" 0 "$(run)"
fi
new_repo
track src/a.rs "// this makes it ${words[3]} for everyone"
check "and the real script refuses the same file" 1 "$(run)"

echo
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
