#!/usr/bin/env bash
#
# Tests for .github/scripts/hygiene/check-commit-identity.sh: only GitHub noreply addresses
# pass, in the author, the Signed-off-by and the Co-authored-by, and the address of
# a rejected commit is never printed.
set -u

HERE="$(cd "$(dirname "$0")/../../../.." && pwd)"
SCRIPT="$HERE/.github/scripts/hygiene/check-commit-identity.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
pass=0; fail=0

check() { # <desc> <want> <got>
	if [ "$2" = "$3" ]; then pass=$((pass + 1)); echo "ok    $1"
	else fail=$((fail + 1)); echo "FAIL  $1 (want '$2', got '$3')"; fi
}

personal="someone@mail.example"
noreply="12345+someone@users.noreply.github.com"

repo="$WORK/repo"
git init -q "$repo"
commit() { # <author email> <message>
	git -C "$repo" -c user.name=t -c "user.email=$1" -c commit.gpgSign=false \
		commit -q --allow-empty -m "$2"
}
commit "$noreply" "base"
base="$(git -C "$repo" rev-parse HEAD)"

run() { (cd "$repo" && "$SCRIPT" "$base" HEAD 2>&1); }

commit "$noreply" "$(printf 'ok\n\nSigned-off-by: t <%s>' "$noreply")"
run >/dev/null; check "noreply author and sign-off pass" 0 "$?"

commit "49699333+dependabot[bot]@users.noreply.github.com" "bot"
run >/dev/null; check "a bot noreply address passes" 0 "$?"

commit "noreply@github.com" "web"
run >/dev/null; check "GitHub's own noreply passes" 0 "$?"

commit "$personal" "personal author"
out="$(run)"; rc=$?
check "a personal author address fails" 1 "$rc"
check "and the address is not printed" 0 "$(printf '%s' "$out" | grep -c "$personal")"
check "and the message names the author field" 1 "$(printf '%s' "$out" | grep -c 'the author address')"

git -C "$repo" reset -q --hard "$base"
commit "$noreply" "$(printf 'x\n\nSigned-off-by: t <%s>' "$personal")"
out="$(run)"; rc=$?
check "a personal Signed-off-by fails" 1 "$rc"
check "and the address is not printed" 0 "$(printf '%s' "$out" | grep -c "$personal")"
check "and the message names the trailer" 1 "$(printf '%s' "$out" | grep -c 'a Signed-off-by address')"

git -C "$repo" reset -q --hard "$base"
commit "$noreply" "$(printf 'x\n\nCo-authored-by: t <%s>' "$personal")"
run >/dev/null; check "a personal Co-authored-by fails" 1 "$?"

git -C "$repo" reset -q --hard "$base"
commit "$noreply" "$(printf 'x\n\nSigned-off-by: t <%s> <%s>' "$personal" "$noreply")"
out="$(run)"; rc=$?
check "a personal address in front of a noreply one fails" 1 "$rc"
check "and the address is not printed" 0 "$(printf '%s' "$out" | grep -c "$personal")"

git -C "$repo" reset -q --hard "$base"
commit "$noreply" "$(printf 'x\n\nSigned-off-by: just a name')"
out="$(run)"; rc=$?
check "a trailer with no address fails" 1 "$rc"
check "and says so" 1 "$(printf '%s' "$out" | grep -c 'with no address')"

git -C "$repo" reset -q --hard "$base"
commit "support@github.com" "older dependabot"
run >/dev/null; check "support@github.com passes" 0 "$?"

git -C "$repo" reset -q --hard "$base"
commit "$noreply" "$(printf 'x\n\nSigned-off-by: t <%s>' "$noreply")"
commit "$personal" "second, personal"
run >/dev/null; check "one bad commit among good ones fails" 1 "$?"

# A key with a space before the colon, and a value folded onto a continuation line, are
# trailers to git, and the address in them is as much a leak as in the plain form.
git -C "$repo" reset -q --hard "$base"
commit "$noreply" "$(printf 'x\n\nSigned-off-by : t <%s>' "$personal")"
out="$(run)"; rc=$?
check "a personal address after a key with a space before the colon fails" 1 "$rc"
check "and the address is not printed" 0 "$(printf '%s' "$out" | grep -c "$personal")"
git -C "$repo" reset -q --hard "$base"
commit "$noreply" "$(printf 'x\n\nSigned-off-by: t\n <%s>' "$personal")"
out="$(run)"; rc=$?
check "a personal address on the continuation line of a folded value fails" 1 "$rc"
check "and the address is not printed" 0 "$(printf '%s' "$out" | grep -c "$personal")"
git -C "$repo" reset -q --hard "$base"
commit "$noreply" "$(printf 'x\n\nco-authored-by : t <%s>' "$personal")"
run >/dev/null; check "a lower-case Co-authored-by with a space before the colon fails" 1 "$?"
git -C "$repo" reset -q --hard "$base"
commit "$noreply" "$(printf 'x\n\nSigned-off-by: t <%s>\n\nmore text after it' "$personal")"
run >/dev/null; check "a sign-off that is not in the last paragraph fails" 1 "$?"
git -C "$repo" reset -q --hard "$base"
commit "$noreply" "$(printf 'x\n\nSigned-off-by : t\n <%s>' "$noreply")"
run >/dev/null; check "a folded value with a noreply address passes" 0 "$?"

# The committer is a second place a personal address lives.
git -C "$repo" reset -q --hard "$base"
GIT_COMMITTER_EMAIL="$personal" commit "$noreply" "ordinary commit, personal committer"
out="$(run)"; rc=$?
check "a noreply author with a personal committer fails on an ordinary commit" 1 "$rc"
check "and the message names the committer field" 1 "$(printf '%s' "$out" | grep -c 'the committer address')"
check "and the address is not printed" 0 "$(printf '%s' "$out" | grep -c "$personal")"
git -C "$repo" reset -q --hard "$base"
GIT_COMMITTER_EMAIL="noreply@github.com" commit "$noreply" "web committer"
run >/dev/null; check "GitHub's own noreply as committer passes" 0 "$?"
git -C "$repo" reset -q --hard "$base"
git -C "$repo" checkout -q -b side
commit "$noreply" "on the side"
git -C "$repo" checkout -q -
git -C "$repo" -c user.name=t -c "user.email=$noreply" -c commit.gpgSign=false commit -q --allow-empty -m "on the trunk"
GIT_COMMITTER_EMAIL="$personal" git -C "$repo" -c user.name=t -c "user.email=$noreply" -c commit.gpgSign=false merge -q --no-ff -m "merge" side
run >/dev/null; check "a merge with a personal committer fails too" 1 "$?"
git -C "$repo" checkout -q -f "$base" 2>/dev/null; git -C "$repo" reset -q --hard "$base"; git -C "$repo" branch -q -D side 2>/dev/null

"$SCRIPT" >/dev/null 2>&1; check "no arguments fails" 1 "$?"

echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
