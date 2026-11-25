#!/usr/bin/env bash
#
# Tests for check-dco.sh: a commit without a sign-off fails, a merge needs none, and a merge
# whose tree is not the clean merge of its parents fails.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
SCRIPT="$HERE/.github/scripts/hygiene/check-dco.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
pass=0; fail=0

check() { # <desc> <want> <got>
	if [ "$2" = "$3" ]; then pass=$((pass + 1)); echo "ok    $1"
	else fail=$((fail + 1)); echo "FAIL  $1 (want '$2', got '$3')"; fi
}

repo="$WORK/repo"
git init -q -b main "$repo"
g() { git -C "$repo" -c user.name=t -c user.email=t@x -c commit.gpgSign=false -c merge.ff=false "$@"; }
run() { (cd "$repo" && bash "$SCRIPT" "$1" "$2" > "$WORK/out" 2>&1; echo $?); }

echo base > "$repo/a"; g add -A; g commit -q -s -m base
base="$(g rev-parse HEAD)"

echo one > "$repo/b"; g add -A; g commit -q -s -m "signed"
check "a signed commit passes" 0 "$(run "$base" HEAD)"

g reset -q --hard "$base"
echo one > "$repo/b"; g add -A; g commit -q -m "unsigned"
check "an unsigned commit fails" 1 "$(run "$base" HEAD)"
grep -q "no Signed-off-by" "$WORK/out"; check "and the output says which" 0 "$?"

g reset -q --hard "$base"
echo one > "$repo/b"; g add -A; g commit -q -s -m "one"
echo two > "$repo/c"; g add -A; g commit -q -m "two, unsigned"
check "one unsigned commit among signed ones fails" 1 "$(run "$base" HEAD)"

g reset -q --hard "$base"
g checkout -q -b side; echo s > "$repo/side"; g add -A; g commit -q -s -m side
g checkout -q main; echo m > "$repo/main"; g add -A; g commit -q -s -m main
main_tip="$(g rev-parse HEAD)"
g merge -q --no-edit side
check "a clean merge needs no trailer of its own" 0 "$(run "$base" HEAD)"

g reset -q --hard "$main_tip"
g merge -q --no-commit --no-ff side; echo smuggled > "$repo/extra"; g add -A; g commit -q -m "merge side"
check "a merge that carries a change of its own fails" 1 "$(run "$base" HEAD)"
grep -q "not the clean merge" "$WORK/out"; check "and the output says why" 0 "$?"

g reset -q --hard "$base"
g checkout -q -b left; echo l > "$repo/x"; g add -A; g commit -q -s -m left
g checkout -q main; g checkout -q -b right; echo r > "$repo/x"; g add -A; g commit -q -s -m right
g merge -q --no-commit --no-ff left >/dev/null 2>&1; echo resolved > "$repo/x"; g add -A; g commit -q -m "merge, resolved by hand"
check "a merge that needed a hand resolution fails closed" 1 "$(run "$base" HEAD)"

# What counts as a sign-off: `Name <address>` with the address of the author. The twin of
# each refusal passes.
sign() { # <message>: a commit by t@x with exactly this message
	g reset -q --hard "$base"
	echo one > "$repo/b"; g add -A; g commit -q -m "$1"
}
sign "$(printf 'x\n\nSigned-off-by:')"
check "an empty Signed-off-by fails" 1 "$(run "$base" HEAD)"
sign "$(printf 'x\n\nSigned-off-by: someone')"
check "a Signed-off-by with no address fails" 1 "$(run "$base" HEAD)"
sign "$(printf 'x\n\nSigned-off-by: someone <someone@elsewhere.example>')"
check "a Signed-off-by for somebody else fails" 1 "$(run "$base" HEAD)"
sign "$(printf 'x\n\nSigned-off-by: <t@x>')"
check "a Signed-off-by with no name fails" 1 "$(run "$base" HEAD)"
sign "$(printf 'x\n\nSigned-off-by: t <t@x> and more')"
check "a Signed-off-by with text after the address fails" 1 "$(run "$base" HEAD)"
sign "$(printf 'x\n\nSigned-off-by: t <t@x>')"
check "a Signed-off-by with the address of the author passes" 0 "$(run "$base" HEAD)"
sign "$(printf 'x\n\nSigned-off-by: T Name <T@X>')"
check "and so does one that differs only in case" 0 "$(run "$base" HEAD)"
sign "$(printf 'x\n\nSigned-off-by: someone <someone@elsewhere.example>\nSigned-off-by: t <t@x>')"
check "one matching sign-off among others passes" 0 "$(run "$base" HEAD)"
sign "$(printf 'x\n\nSigned-off-by: t\n <t@x>')"
check "a value folded onto the next line passes" 0 "$(run "$base" HEAD)"

# The one exception: the commit GitHub writes for a Dependabot pull request. The pair of author
# and trailer is accepted as a whole, and each attack on it is refused.
dep_author='49699333+dependabot[bot]@users.noreply.github.com'
dep_signoff='dependabot[bot] <support@github.com>'
as_author() { # <name> <email> <message>: a commit by this author with exactly this message
	g reset -q --hard "$base"
	echo one > "$repo/b"; g add -A
	git -C "$repo" -c user.name="$1" -c user.email="$2" -c commit.gpgSign=false commit -q -m "$3"
}
as_author dependabot[bot] "$dep_author" "$(printf 'x\n\nSigned-off-by: %s' "$dep_signoff")"
check "Dependabot's author with the sign-off GitHub puts on it passes" 0 "$(run "$base" HEAD)"
as_author dependabot[bot] "$dep_author" "$(printf 'x\n\nSigned-off-by: dependabot[bot] <%s>' "$dep_author")"
check "Dependabot's author signing with its own address still passes" 0 "$(run "$base" HEAD)"
as_author someone someone@elsewhere.example "$(printf 'x\n\nSigned-off-by: %s' "$dep_signoff")"
check "another author with Dependabot's trailer fails" 1 "$(run "$base" HEAD)"
as_author dependabot[bot] "$dep_author" "$(printf 'x\n\nSigned-off-by: dependabot[bot] <someone@elsewhere.example>')"
check "Dependabot's author with another address in the trailer fails" 1 "$(run "$base" HEAD)"
as_author dependabot[bot] "$dep_author" "$(printf 'x\n\nSigned-off-by: Dependabot <support@github.com>')"
check "Dependabot's author with another name on the trailer fails" 1 "$(run "$base" HEAD)"
as_author dependabot[bot] "$dep_author" "$(printf 'x\n\nSigned-off-by: %s and more' "$dep_signoff")"
check "Dependabot's trailer with text after it fails" 1 "$(run "$base" HEAD)"
as_author dependabot[bot] "$dep_author" "$(printf 'x\n\nSigned-off-by: dependabot[bot] <SUPPORT@github.com>')"
check "the trailer is compared as written, not in another case" 1 "$(run "$base" HEAD)"
as_author dependabot[bot] "12345+dependabot[bot]@users.noreply.github.com" "$(printf 'x\n\nSigned-off-by: %s' "$dep_signoff")"
check "an author address that only looks like Dependabot's fails" 1 "$(run "$base" HEAD)"
as_author dependabot[bot] "49699333+Dependabot[bot]@users.noreply.github.com" "$(printf 'x\n\nSigned-off-by: %s' "$dep_signoff")"
check "and so does one that differs from it in case" 1 "$(run "$base" HEAD)"
as_author t t@x "$(printf 'x\n\nSigned-off-by: t <support@github.com>')"
check "a person signing with support@github.com fails" 1 "$(run "$base" HEAD)"
as_author dependabot[bot] "$dep_author" "x"
check "Dependabot's author with no sign-off at all fails" 1 "$(run "$base" HEAD)"

check "an unreadable range fails" 1 "$(run "$base" nonexistent)"
check "no arguments fails" 1 "$( (cd "$repo" && bash "$SCRIPT" > /dev/null 2>&1; echo $?) | sed 's/^2$/1/')"

echo "$pass passed, $fail failed"
echo "DONE check-dco.test.sh $pass $fail"
[ "$fail" -eq 0 ]
