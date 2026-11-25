#!/usr/bin/env bash
#
# .github/scripts/hygiene/check-pins.py: the value of every pin, checked on its own because
# the wiring contract does not hold it. The repositories the pins point to are local bare
# repositories (PINS_REMOTE), so nothing here needs the network.
#
# Requires: python3 with PyYAML, git.
# shellcheck disable=SC2016
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
SCRIPT="$HERE/.github/scripts/hygiene/check-pins.py"
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

export GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@users.noreply.github.com
export GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@users.noreply.github.com
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null

# <owner>/<repo>: tags v1.0.0 (lightweight, commit 1), v1.1.0 (annotated, commit 2),
# v2.0.0 (lightweight, commit 3). Commit 4 is on a branch only: what a fork could add.
make_remote() {
	local repo="$WORK/remote/$1" src
	src="$WORK/src/$1"
	mkdir -p "$src" "$(dirname "$repo")"
	git -C "$src" init -q -b main
	for n in 1 2 3; do
		echo "$n" >"$src/f"
		git -C "$src" add f
		git -C "$src" commit -q -m "c$n"
		case "$n" in
		1) git -C "$src" tag v1.0.0 ;;
		2) git -C "$src" tag -a v1.1.0 -m "annotated" ;;
		3) git -C "$src" tag v2.0.0 ;;
		esac
	done
	git -C "$src" checkout -q -b side
	echo 4 >"$src/f"
	git -C "$src" commit -q -am c4
	git clone -q --bare "$src" "$repo"
}
make_remote actions/checkout
make_remote actions/cache
make_remote evil/thing
rev() { git -C "$WORK/src/$1" rev-parse "$2^{commit}"; }
C1="$(rev actions/checkout v1.0.0)"
C2="$(rev actions/checkout v1.1.0)"
C3="$(rev actions/checkout v2.0.0)"
C4="$(git -C "$WORK/src/actions/checkout" rev-parse side)"
K3="$(rev actions/cache v2.0.0)"
export PINS_REMOTE="file://$WORK/remote"

DEB="debian:trixie@sha256:$(printf 'a%.0s' $(seq 64))"
tree=0
# <name> [files as "path<TAB>content" given by a here-doc to write_tree]
new_tree() {
	tree=$((tree + 1))
	T="$WORK/t$tree"
	mkdir -p "$T/.github/workflows"
}
step() { # <uses with its comment> -> a one step job file
	printf 'on: push\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: %s\n' "$1"
}
run() { # <head> [<base>] -> exit code; the output stays in $WORK/out
	(cd "$WORK" && python3 -I "$SCRIPT" --head "$1" ${2:+--base "$2"} >"$WORK/out" 2>&1)
	echo $?
}
saw() { grep -c -- "$1" "$WORK/out"; }
one() { # <description> <expected exit> <uses line> [<message needle>]
	new_tree
	step "$3" >"$T/.github/workflows/a.yml"
	check "$1" "$2" "$(run "$T")"
	[ -n "${4:-}" ] && check "$1: says why" 1 "$(saw "$4")"
}

one "a commit that is the tag, lightweight" 0 "actions/checkout@$C1 # v1.0.0"
one "a commit that is the tag, annotated: the peeled commit counts" 0 "actions/checkout@$C2 # v1.1.0"
one "an action in a path of the repository" 0 "actions/checkout/sub/dir@$C1 # v1.0.0"
one "the commit of another tag" 1 "actions/checkout@$C3 # v1.0.0" "is not the commit of v1.0.0"
one "the commit of the annotated tag object instead of its commit" 1 "actions/checkout@$(git -C "$WORK/src/actions/checkout" rev-parse v1.1.0) # v1.1.0" "is not the commit of v1.1.0"
one "a commit that only a branch has (what a fork could add)" 1 "actions/checkout@$C4 # v1.0.0" "is not the commit of v1.0.0"
one "a tag the repository does not have (it cannot be confirmed, which also fails)" 2 "actions/checkout@$C1 # v9.9.9" "cannot find the commit of v9.9.9"
one "an owner that is not on the list" 1 "evil/thing@$(rev evil/thing v1.0.0) # v1.0.0" "is not one of"
one "a tag in place of the commit" 1 "actions/checkout@v1.0.0 # v1.0.0" "not pinned to a full"
one "a branch in place of the commit" 1 "actions/checkout@main # v1.0.0" "not pinned to a full"
one "a short hash" 1 "actions/checkout@${C1:0:39} # v1.0.0" "not pinned to a full"
one "an upper-case hash" 1 "actions/checkout@$(echo "$C1" | tr a-f A-F) # v1.0.0" "not pinned to a full"
one "no pin" 1 "actions/checkout # v1.0.0" "is not \`<owner>/<repo>@<commit>\`"
one "no version comment" 1 "actions/checkout@$C1" "is not written as"
one "a comment that is not three numbers" 1 "actions/checkout@$C1 # v1" "is not written as"
one "a comment without the v" 1 "actions/checkout@$C1 # 1.0.0" "is not written as"
one "a comment with a suffix" 1 "actions/checkout@$C1 # v1.0.0-rc1" "is not written as"
one "a leading zero in the version" 1 "actions/checkout@$C1 # v01.0.0" "is not written as"

new_tree
printf 'on: push\njobs:\n  a:\n    runs-on: x\n    steps:\n      - { uses: "actions/checkout@%s" } # v1.0.0\n' "$C1" >"$T/.github/workflows/a.yml"
check "a reference in a flow mapping, which the line pattern does not see, is refused" 1 "$(run "$T")"
new_tree
printf 'on: push\njobs:\n  a:\n    runs-on: x\n    steps:\n      - uses: actions/checkout@%s # v1.0.0\n        uses: actions/cache@%s # v2.0.0\n' "$C1" "$K3" >"$T/.github/workflows/a.yml"
check "a duplicate key cannot be read, so it fails" 2 "$(run "$T")"
new_tree
printf 'on: push\njobs:\n  a:\n    runs-on: x\n    steps:\n      - run: |\n          echo "uses: actions/checkout@%s # v1.0.0"\n      - uses: actions/cache@%s # v2.0.0\n' "$C1" "$K3" >"$T/.github/workflows/a.yml"
check "a line in a script that looks like a reference does not stand in for a real one" 0 "$(run "$T")"
new_tree
printf 'on: push\njobs:\n  a:\n    runs-on: x\n    steps:\n      - uses: actions/checkout@%s\n      - run: |\n          uses: actions/checkout@%s # v1.0.0\n' "$C1" "$C1" >"$T/.github/workflows/a.yml"
check "a commentless reference with a look-alike line in a script is refused" 1 "$(run "$T")"
new_tree
mkdir -p "$T/x"
printf 'name: x\nruns:\n  using: composite\n  steps:\n    - uses: actions/checkout@%s # v1.0.0\n      shell: bash\n' "$C3" >"$T/x/action.yml"
check "a composite action is read too" 1 "$(run "$T")"
new_tree
printf 'on: push\njobs:\n  a:\n    runs-on: x\n    steps:\n      - uses: ./.github/actions/x\n' >"$T/.github/workflows/a.yaml"
check "a local reference and a .yaml file need no pin" 0 "$(run "$T")"

new_tree
step "actions/checkout@$C1 # v1.0.0" | sed 's/$/\r/' >"$T/.github/workflows/a.yml"
check "a file with CRLF line ends is read the same" 0 "$(run "$T")"
new_tree
step "actions/checkout@$C1 # v1.0.0" | tr '\n' '\r' >"$T/.github/workflows/a.yml"
check "a file whose lines end in a lone CR is refused, its lines would not be counted as YAML counts them" 2 "$(run "$T")"

# One pair per action across the whole tree.
new_tree
step "actions/checkout@$C1 # v1.0.0" >"$T/.github/workflows/a.yml"
step "actions/checkout@$C3 # v2.0.0" >"$T/.github/workflows/b.yml"
check "two pairs of one action in two files" 1 "$(run "$T")"
check "...and the message names the action" 1 "$(saw 'more than one pair')"
new_tree
step "actions/checkout@$C1 # v1.0.0" >"$T/.github/workflows/a.yml"
step "actions/checkout@$C1 # v1.0.0" >"$T/.github/workflows/b.yml"
step "actions/cache@$K3 # v2.0.0" >"$T/.github/workflows/c.yml"
check "the same pair in several files, and another action" 0 "$(run "$T")"

# Against the base: a version does not go down.
new_tree
BASE="$T"
step "actions/checkout@$C3 # v2.0.0" >"$BASE/.github/workflows/a.yml"
new_tree
step "actions/checkout@$C1 # v1.0.0" >"$T/.github/workflows/a.yml"
check "a version below the one the base pins" 1 "$(run "$T" "$BASE")"
check "...and the message says it goes down" 1 "$(saw 'goes down to v1.0.0')"
check "the same pins against themselves" 0 "$(run "$BASE" "$BASE")"
new_tree
step "actions/cache@$K3 # v2.0.0" >"$T/.github/workflows/a.yml"
check "an action the base does not have" 0 "$(run "$T" "$BASE")"
new_tree
step "actions/checkout@$C3 # v2.0.0" >"$T/.github/workflows/a.yml"
NEW="$T"
new_tree
step "actions/checkout@$C1 # v1.0.0" >"$T/.github/workflows/a.yml"
check "a version above the one the base pins" 0 "$(run "$NEW" "$T")"
mkdir -p "$WORK/empty"
check "a base with no workflows at all asks nothing of the head" 0 "$(run "$NEW" "$WORK/empty")"
new_tree
printf 'on: [push\n' >"$T/.github/workflows/a.yml"
check "a base that cannot be read fails, it is not taken for absent" 2 "$(run "$NEW" "$T")"

# Images.
img() { # <description> <expected> <yaml image line> [needle]
	new_tree
	printf 'on: push\njobs:\n  a:\n    runs-on: x\n    container: %s\n    steps:\n      - run: true\n' "$3" >"$T/.github/workflows/a.yml"
	check "$1" "$2" "$(run "$T")"
	[ -n "${4:-}" ] && check "$1: says why" 1 "$(saw "$4")"
}
img "an image by digest from the list" 0 "$DEB"
img "an image by tag only" 1 "debian:trixie" "is not \`<repository>"
img "an image with a short digest" 1 "debian:trixie@sha256:abc" "is not \`<repository>"
img "an image from another repository" 1 "evilcorp/debian:trixie@sha256:$(printf 'a%.0s' $(seq 64))" "is not one of"
img "an image taken from an input is judged where it is written" 0 '${{ inputs.image }}'
new_tree
printf 'on:\n  workflow_call:\n    inputs:\n      i:\n        type: string\n        default: "evil/x:1@sha256:%s"\n' "$(printf 'b%.0s' $(seq 64))" >"$T/.github/workflows/a.yml"
check "an input default that carries a digest of another repository" 1 "$(run "$T")"
new_tree
step "docker://alpine:3@sha256:$(printf 'c%.0s' $(seq 64))" >"$T/.github/workflows/a.yml"
check "a docker:// step from a repository that is not listed" 1 "$(run "$T")"
new_tree
printf 'on: push\njobs:\n  a:\n    runs-on: x\n    container: %s\n    steps:\n      - run: true\n  b:\n    runs-on: x\n    container: debian:trixie@sha256:%s\n    steps:\n      - run: true\n' "$DEB" "$(printf 'd%.0s' $(seq 64))" >"$T/.github/workflows/a.yml"
check "one image tag with two digests" 1 "$(run "$T")"

# The address is not rewritten by local configuration, and a remote that cannot be asked fails.
new_tree
step "actions/checkout@$C1 # v1.0.0" >"$T/.github/workflows/a.yml"
mkdir -p "$WORK/evil/actions"
git clone -q --bare "$WORK/src/actions/checkout" "$WORK/evil/actions/checkout"
git -C "$WORK/evil/actions/checkout" tag -f v1.0.0 "$C3" >/dev/null 2>&1
printf '[url "file://%s/evil/"]\n\tinsteadOf = file://%s/remote/\n' "$WORK" "$WORK" >"$WORK/gitconfig"
check "the evil copy has the tag elsewhere (control: asked directly)" 1 "$(PINS_REMOTE="file://$WORK/evil" run "$T")"
check "a rewrite in the global git configuration is not followed" 0 "$(GIT_CONFIG_GLOBAL="$WORK/gitconfig" HOME="$WORK" run "$T")"
check "a rewrite in the configuration of the working directory's repository is not followed" 0 "$(cd "$WORK" && git init -q rw 2>/dev/null; cd "$WORK/rw" && git config url."file://$WORK/evil/".insteadOf "file://$WORK/remote/" && (python3 -I "$SCRIPT" --head "$T" >"$WORK/out" 2>&1; echo $?))"
check "a remote that does not answer fails" 2 "$(PINS_REMOTE="file://$WORK/nowhere" run "$T")"
check "...and says that it could not ask" 1 "$(saw 'cannot find the commit')"
check "no output on success says what was checked" 1 "$(run "$BASE" >/dev/null; saw 'in order')"

# The tree as it is: every pin of the repository is in order when the network is there.
if git ls-remote https://github.com/actions/checkout refs/tags/v7.0.1 >/dev/null 2>&1; then
	check "the repository's own pins pass against the real remotes" 0 "$(unset PINS_REMOTE; run "$HERE" "$HERE")"
fi

echo
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
