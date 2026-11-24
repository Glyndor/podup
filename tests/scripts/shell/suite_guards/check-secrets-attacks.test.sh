#!/usr/bin/env bash
#
# Two ways the secret scan used to read nothing and say so, attacked on the shared copy:
#
#   1. A file with a NUL and bytes that are not UTF-8, sorted before a file with a token,
#      turned the diff into a stream that grep and awk treat as binary, and a `|| true`
#      made "could not read" the same as "found nothing".
#   2. The root commit of an orphan branch has no parent, so `cut` printed the commit
#      itself as its parent and the diff against the empty tree never ran: a root commit
#      that adds a token and a second commit that removes it, merged, was never seen.
#
# Each case builds a real repository. The token is assembled at run time, so this file does
# not trip the check it tests, and the scan must never print the value.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
SCRIPT="$HERE/.github/scripts/hygiene/check-secrets.sh"
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

rep() { printf '%*s' "$2" '' | tr ' ' "$1"; } # <char> <count>
token="gh$(printf 'p')_$(rep a 36)"

repo="$WORK/repo"
git init -q -b main "$repo"
gitc() { git -C "$repo" -c user.name=t -c user.email=t@users.noreply.github.com -c commit.gpgSign=false "$@"; }
echo base >"$repo/README"
gitc add -A
gitc commit -q -m base
base="$(git -C "$repo" rev-parse HEAD)"
scan() { (cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1); }
reset() {
	git -C "$repo" checkout -q -f main && git -C "$repo" reset -q --hard "$base" && git -C "$repo" clean -q -fdx
	git -C "$repo" branch -q -D side alone 2>/dev/null || true
}

# 1. Input that is not text, before a file that holds a token.
reset
mkdir "$repo/dir"
printf 'x\000\377\377y\n' >"$repo/dir/aaa.bin"
printf 't=%s\n' "$token" >"$repo/dir/zzz.txt"
gitc add -A
gitc commit -q -m "binary first, token after"
out="$(scan)"
rc=$?
check "a file with a NUL and 0xFF that sorts before a token file: the token is found" 1 "$rc"
check "...and named" 1 "$(printf '%s' "$out" | grep -c 'GitHub token')"
check "...without printing the value" 0 "$(printf '%s' "$out" | grep -cF "$token")"

reset
printf '\377\376\377 not utf-8\n' >"$repo/aaa.dat"
printf 't=%s\n' "$token" >"$repo/zzz.txt"
gitc add -A
gitc commit -q -m "invalid bytes first, token after"
scan >/dev/null
check "a file of bytes that are not UTF-8 before a token file: the token is found" 1 "$?"

reset
printf '\000\000\000\000' >"$repo/aaa.bin"
printf 'harmless\n' >"$repo/zzz.txt"
gitc add -A
gitc commit -q -m "binary and harmless"
scan >/dev/null
check "(control) the same binary file beside a harmless one passes" 0 "$?"

reset
printf 't=%s\n' "$token" >"$repo/aaa.txt"
printf 'x\000\377\377y\n' >"$repo/zzz.bin"
gitc add -A
gitc commit -q -m "token first, binary after"
scan >/dev/null
check "a token before a file that is not text is found as well" 1 "$?"

# 2. A merged orphan branch: the root commit adds a token and the next one removes it.
orphan() { # <add the token to the root commit: yes|no>
	reset
	git -C "$repo" checkout -q --orphan side
	git -C "$repo" rm -q -rf . 2>/dev/null
	if [ "$1" = yes ]; then printf 't=%s\n' "$token" >"$repo/orphan.txt"; else printf 'plain\n' >"$repo/orphan.txt"; fi
	gitc add -A
	gitc commit -q -m "root of the orphan branch"
	printf 'clean\n' >"$repo/orphan.txt"
	gitc add -A
	gitc commit -q -m "second commit of the orphan branch"
	git -C "$repo" checkout -q main
	gitc merge -q --allow-unrelated-histories --no-ff -m "merge the orphan branch" side >/dev/null 2>&1
}
orphan yes
check "(the merge exists) the merge commit has two parents" 2 "$(git -C "$repo" rev-list --parents -n 1 HEAD | awk '{print NF - 1}')"
check "(the root is a root) the orphan's first commit has no parent" 1 "$(git -C "$repo" rev-list --parents --max-parents=0 side | awk '{print NF}' | sort -u | tr -d '\n')"
out="$(scan)"
rc=$?
check "a root commit that adds a token and a second that removes it, merged, is found" 1 "$rc"
check "...and named" 1 "$(printf '%s' "$out" | grep -c 'GitHub token')"
check "...without printing the value" 0 "$(printf '%s' "$out" | grep -cF "$token")"
orphan no
scan >/dev/null
check "(control) the same orphan branch with no token passes" 0 "$?"

# A root commit that is the whole range: base is the empty tree's neighbour in a new repository.
reset
git -C "$repo" checkout -q --orphan alone
git -C "$repo" rm -q -rf . 2>/dev/null
printf 't=%s\n' "$token" >"$repo/alone.txt"
gitc add -A
gitc commit -q -m "a branch of its own"
(cd "$repo" && "$SCRIPT" "$base" HEAD >/dev/null 2>&1)
check "an orphan branch compared with the base, token in its only commit, is found" 1 "$?"

echo
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
