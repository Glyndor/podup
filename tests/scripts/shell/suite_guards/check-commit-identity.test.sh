#!/usr/bin/env bash
#
# Tests for .github/scripts/hygiene/check-commit-identity.sh: only GitHub noreply addresses
# pass, in the author, the Signed-off-by and the Co-authored-by, and the address of
# a rejected commit is never printed.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
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

# A trailer in a paragraph that is not the last is a trailer too: the lines of the whole
# message are read, in any position and in any case. Each refusal has a twin with a noreply
# address in the same place that passes.
for key in Signed-off-by Co-authored-by signed-off-by CO-AUTHORED-BY; do
	git -C "$repo" reset -q --hard "$base"
	commit "$noreply" "$(printf 'x\n\n%s: t <%s>\n\nthe rest of the message, a last paragraph' "$key" "$personal")"
	out="$(run)"; rc=$?
	check "a personal $key in a paragraph that is not the last fails" 1 "$rc"
	check "and the message names the trailer ($key)" 1 "$(printf '%s' "$out" | grep -c -i 'a \(Signed-off-by\|Co-authored-by\) address')"
	check "and the address is not printed ($key)" 0 "$(printf '%s' "$out" | grep -c "$personal")"
	git -C "$repo" reset -q --hard "$base"
	commit "$noreply" "$(printf 'x\n\n%s: t <%s>\n\nthe rest of the message, a last paragraph' "$key" "$noreply")"
	run >/dev/null; check "a noreply $key in a paragraph that is not the last passes" 0 "$?"
done
git -C "$repo" reset -q --hard "$base"
commit "$noreply" "$(printf 'x\n\nSigned-off-by: t <%s>\n\nbetween\n\nSigned-off-by: u <%s>\n' "$personal" "$noreply")"
run >/dev/null; check "a personal sign-off above a noreply one fails" 1 "$?"
git -C "$repo" reset -q --hard "$base"
commit "$noreply" "$(printf 'x\n\nnote that Signed-off-by: t <%s> is not a trailer when it is not at the start of a line\n' "$personal")"
run >/dev/null; check "a line that only mentions the key mid-sentence is not read" 0 "$?"

# A merge commit in the range is checked like the rest: its author, its committer, and the
# trailers of its message. The merge GitHub makes has GitHub's own address as committer and
# passes. merge <author> <committer> <message>: a side branch merged without fast-forward.
merge() {
	git -C "$repo" reset -q --hard "$base"
	git -C "$repo" checkout -q -b side
	commit "$noreply" "a side commit"
	git -C "$repo" checkout -q -
	GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL="$1" GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL="$2" \
		git -C "$repo" -c commit.gpgSign=false merge -q --no-ff -m "$3" side
	git -C "$repo" branch -q -D side
}
merge "$noreply" "$noreply" "merge the side branch"
out="$(run)"; rc=$?
check "a merge by a noreply author and committer passes" "0 1" "$rc $(printf '%s' "$out" | grep -c 'every commit uses a GitHub noreply address')"
merge "$noreply" "noreply@github.com" "Merge branch 'main' into side"
run >/dev/null; check "a merge GitHub made, committed by its own noreply address, passes" 0 "$?"
merge "$personal" "$noreply" "merge the side branch"
out="$(run)"; rc=$?
check "a merge with a personal author fails" 1 "$rc"
check "and the message names the author" 1 "$(printf '%s' "$out" | grep -c 'the author address')"
check "and the address is not printed" 0 "$(printf '%s' "$out" | grep -c "$personal")"
merge "$noreply" "$personal" "merge the side branch"
out="$(run)"; rc=$?
check "a merge with a personal committer fails" 1 "$rc"
check "and the message names the committer address" 1 "$(printf '%s' "$out" | grep -c 'the committer address')"
check "and the address is not printed" 0 "$(printf '%s' "$out" | grep -c "$personal")"
merge "$noreply" "$noreply" "$(printf 'merge\n\nSigned-off-by: t <%s>\n' "$personal")"
out="$(run)"; rc=$?
check "a merge with a personal Signed-off-by fails" 1 "$rc"
check "and the message names the trailer" 1 "$(printf '%s' "$out" | grep -c 'a Signed-off-by address')"
git -C "$repo" reset -q --hard "$base"

# A range that cannot be listed is a failure, not a clean pass; the same script over
# a range that can be listed, with nothing in it, still passes.
out="$( (cd "$repo" && "$SCRIPT" 0000000000000000000000000000000000000000 HEAD 2>&1) )"; rc=$?
check "a range that cannot be listed fails" 1 "$rc"
check "and says the range could not be checked" 1 "$(printf '%s' "$out" | grep -c 'could not list 0000000000000000000000000000000000000000..HEAD; the range could not be checked')"
check "and gives no clean verdict" 0 "$(printf '%s' "$out" | grep -c 'every commit uses')"
out="$( (cd "$repo" && "$SCRIPT" HEAD HEAD 2>&1) )"; rc=$?
check "an empty range that can be listed passes" 0 "$rc"
check "and gives the clean verdict" 1 "$(printf '%s' "$out" | grep -c 'every commit uses a GitHub noreply address')"

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




# A trailer with nothing after the key is not a trailer with nothing to check.
git -C "$repo" reset -q --hard "$base"
commit "$noreply" "$(printf 'x\n\nSigned-off-by:')"
out="$(run)"; rc=$?
check "an empty Signed-off-by fails" 1 "$rc"
check "and the message says it has no value" 1 "$(printf '%s' "$out" | grep -c 'no value')"
git -C "$repo" reset -q --hard "$base"
commit "$noreply" "$(printf 'x\n\nCo-authored-by:   ')"
run >/dev/null; check "an empty Co-authored-by fails" 1 "$?"
git -C "$repo" reset -q --hard "$base"
commit "$noreply" "$(printf 'x\n\nSigned-off-by:\n <%s>' "$noreply")"
run >/dev/null; check "a value on the next line is not an empty one" 0 "$?"

"$SCRIPT" >/dev/null 2>&1; check "no arguments fails" 1 "$?"

echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
