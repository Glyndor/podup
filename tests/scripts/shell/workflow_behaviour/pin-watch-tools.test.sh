#!/usr/bin/env bash
#
# Behaviour tests for .github/scripts/checks/pin-watch-tools.sh, which reads the
# tool versions pinned in the workflows and compares each with the tool's latest
# release.
#
# The script runs inside a fixture repository: its own copy, one small workflow
# file per pin site with a version chosen here, and a stand-in `gh` that answers
# `release view` and `release list` from a table. Nothing reaches the network,
# and nothing depends on the versions the real workflows pin today.
#
# Every rule has a refusal and an acceptance of the same shape: a pin that cannot
# be read fails the run, a tool declared without a pin site fails it, a pin site
# whose tool is not declared fails it, a pin in the workflows that no table lists
# fails it, a release that cannot be read fails it, and sites of one tool that
# disagree are reported on their own. The last case runs the tables as they ship
# against the real workflows, so a site that moved or a shape that stopped
# matching is found here and not on a Monday.
#
# Requires: bash, awk, GNU sort (for -V).
set -uo pipefail
export LC_ALL=C

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
SCRIPT_REL=".github/scripts/checks/pin-watch-tools.sh"
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

# A stand-in gh. UPSTREAM is a file of `repo<TAB>latest tag<TAB>space-separated tags`;
# a repo missing from it, or listed with the tag `FAIL`, answers with an error.
# Every call is logged.
mkdir -p "$WORK/bin"
cat >"$WORK/bin/gh" <<'GH'
#!/usr/bin/env bash
echo "$*" >>"$GH_LOG"
[ "$1" = release ] || exit 2
verb="$2"
repo=""
while [ $# -gt 0 ]; do
	[ "$1" = --repo ] && repo="$2"
	shift
done
row=$(awk -F'\t' -v r="$repo" '$1 == r' "$UPSTREAM")
[ -n "$row" ] || exit 1
latest=$(printf '%s' "$row" | cut -f2)
[ "$latest" != FAIL ] || exit 1
case "$verb" in
	view) echo "$latest" ;;
	list) printf '%s' "$row" | cut -f3 | tr ' ' '\n' ;;
esac
GH
chmod +x "$WORK/bin/gh"

# One input of a reusable workflow, written the way the real ones are.
input() { # <key> <default>
	printf '    inputs:\n      %s:\n        description: x\n        type: string\n        default: "%s"\n' "$1" "$2"
}

# The fixture repository, in $1: the script, and one file per pin site.
build_repo() {
	local r="$1" w="$1/.github/workflows"
	mkdir -p "$w" "$r/.github/scripts/checks"
	cp "$HERE/$SCRIPT_REL" "$r/$SCRIPT_REL"
	{ echo "on:"; echo "  workflow_call:"; input llvm-cov-version 0.8.7 | sed 1d; input semver-checks-version 0.50.0 | sed 1d; } >"$w/reusable-rust-ci.yml"
	{ echo "on:"; echo "  workflow_call:"; input cargo-audit-version 0.22.2 | sed 1d; input cargo-deny-version 0.20.2 | sed 1d; } >"$w/reusable-rust-audit.yml"
	{ echo "on:"; echo "  workflow_call:"; input cargo-fuzz-version 0.13.2 | sed 1d; } >"$w/reusable-rust-fuzz.yml"
	{ echo "on:"; echo "  workflow_call:"; input cyclonedx-version 0.5.9 | sed 1d; input cargo-about-version 0.9.2 | sed 1d; } >"$w/reusable-rust-supply-chain.yml"
	{ echo "on:"; echo "  workflow_call:"; input shellcheck-version v0.11.0 | sed 1d; } >"$w/reusable-shell-ci.yml"
	{ echo "on:"; echo "  workflow_call:"; input shellcheck-version v0.11.0 | sed 1d; echo "env:"; echo "  ACTIONLINT_VERSION: v1.7.12"; } >"$w/reusable-workflow-lint.yml"
	{ echo "on:"; echo "  workflow_call:"; input pssa-version 1.25.0 | sed 1d; } >"$w/reusable-powershell-ci.yml"
	cat >"$w/release.yml" <<'YML'
jobs:
  x:
    steps:
      - run: |
          cargo install cargo-audit --version "=0.22.2" --locked
          cargo install --locked cargo-cyclonedx --version "=0.5.9"
          cargo install --locked --features cli cargo-about --version "=0.9.2"
YML
}

# What every tool's repository says is latest, matching the fixture pins.
write_upstream() {
	printf '%s\t%s\t%s\n' \
		taiki-e/cargo-llvm-cov v0.8.7 "v0.8.7 v0.8.6" \
		obi1kenobi/cargo-semver-checks v0.50.0 "v0.50.0" \
		rustsec/cargo-audit cvss/v3.0.0 "cvss/v3.0.0 platforms/v4.2.1 cargo-audit/v0.22.2 cargo-audit/v0.22.1" \
		EmbarkStudios/cargo-deny 0.20.2 "0.20.2" \
		rust-fuzz/cargo-fuzz 0.13.2 "0.13.2" \
		CycloneDX/cyclonedx-rust-cargo cargo-cyclonedx-0.5.9 "cargo-cyclonedx-0.5.9 cyclonedx-bom-0.8.1" \
		EmbarkStudios/cargo-about 0.9.2 "0.9.2" \
		koalaman/shellcheck v0.11.0 "v0.11.0" \
		rhysd/actionlint v1.7.12 "v1.7.12" \
		PowerShell/PSScriptAnalyzer 1.25.0 "1.25.0" >"$1"
}

# Run the fixture's script. Sets RC, OUT (stdout) and ERR (stderr).
run() { # <repo dir> <upstream file>
	: >"$WORK/gh.log"
	GH_LOG="$WORK/gh.log" UPSTREAM="$2" PATH="$WORK/bin:$PATH" \
		bash "$1/$SCRIPT_REL" >"$WORK/out" 2>"$WORK/err"
	RC=$?
	OUT=$(cat "$WORK/out")
	ERR=$(cat "$WORK/err")
}

fresh() { # <name>: a clean fixture in $WORK/<name>, upstream in $WORK/<name>.up
	rm -rf "${WORK:?}/$1"
	build_repo "$WORK/$1"
	write_upstream "$WORK/$1.up"
}

# --- no drift ---------------------------------------------------------------
fresh a
run "$WORK/a" "$WORK/a.up"
check "pins equal to the latest releases: success" 0 "$RC"
check "...and no row on stdout" "" "$OUT"
check "...reporting the size of what it watched" 1 "$(grep -c 'watched 10 tools over 14 pin sites' "$WORK/err")"
check "...through gh release view and list only, never the API" 0 "$(grep -vcE '^release (view|list) ' "$WORK/gh.log")"

# --- drift, with and without a prefix on the tag ----------------------------
fresh b
sed -i 's|^taiki-e/cargo-llvm-cov\tv0.8.7|taiki-e/cargo-llvm-cov\tv0.9.1|' "$WORK/b.up"
run "$WORK/b" "$WORK/b.up"
check "a tool behind its latest release is reported" 1 "$(printf '%s\n' "$OUT" | grep -c '^cargo-llvm-cov (llvm-cov-version)')"
check "...with the pin and the latest on the row" "cargo-llvm-cov (llvm-cov-version)	.github/workflows/reusable-rust-ci.yml	0.8.7	0.9.1" "$OUT"
check "...and the run still succeeds" 0 "$RC"

fresh c
printf '%s\t%s\t%s\n' rustsec/cargo-audit cvss/v3.0.0 "cvss/v3.0.0 cargo-audit/v0.23.0 cargo-audit/v0.22.2" >>"$WORK/c.up"
grep -v '^rustsec/cargo-audit	cvss/v3.0.0	cvss/v3.0.0 platforms' "$WORK/c.up" >"$WORK/c.up2"
run "$WORK/c" "$WORK/c.up2"
check "a newest release that is not a version of the tool is skipped for the newest one that is (both sites)" 2 "$(printf '%s\n' "$OUT" | grep -c 'cargo-audit.*0.22.2.0.23.0')"

# a prefixed tag is told back in the form the file holds
fresh d
sed -i 's|^shellcheck-version.*||; s|"v0.11.0"|"v0.11.0"|' "$WORK/d/.github/workflows/reusable-shell-ci.yml"
sed -i 's|^koalaman/shellcheck\tv0.11.0|koalaman/shellcheck\tv0.12.0|' "$WORK/d.up"
run "$WORK/d" "$WORK/d.up"
check "a pin that holds the tag with its v is told the newer tag with its v (both sites)" 2 "$(printf '%s\n' "$OUT" | grep -c 'v0.11.0.v0.12.0$')"

# --- sites of one tool that disagree ----------------------------------------
fresh e
sed -i 's|=0.9.2|=0.9.0|' "$WORK/e/.github/workflows/release.yml"
run "$WORK/e" "$WORK/e.up"
check "two sites of one tool with different versions are reported as a disagreement" 1 "$(printf '%s\n' "$OUT" | grep -c 'cargo-about (2 sites disagree)')"
check "...and the site that is behind is reported against the latest" 1 "$(printf '%s\n' "$OUT" | grep -c 'cargo-about (install).*0.9.0.0.9.2$')"

# --- a pin that cannot be read ------------------------------------------------
fresh f
sed -i '/default:/d' "$WORK/f/.github/workflows/reusable-rust-audit.yml"
run "$WORK/f" "$WORK/f.up"
check "an input that lost its default fails the run" 1 "$RC"
check "...naming the tool, the key and the file" 1 "$(printf '%s\n' "$ERR" | grep -c 'could not read the cargo-audit pin (cargo-audit-version) from .github/workflows/reusable-rust-audit.yml')"
check "...and prints no table" "" "$OUT"

fresh g
# shellcheck disable=SC2016
sed -i 's|--version "=0.9.2"|--version "$ABOUT"|' "$WORK/g/.github/workflows/release.yml"
run "$WORK/g" "$WORK/g.up"
check "an install line whose version is not a literal fails the run" 1 "$RC"

fresh h
rm "$WORK/h/.github/workflows/reusable-rust-fuzz.yml"
run "$WORK/h" "$WORK/h.up"
check "a pin site whose file is gone fails the run" 1 "$RC"
check "...saying the file does not exist" 1 "$(printf '%s\n' "$ERR" | grep -c 'does not exist')"

# --- the two tables against each other ----------------------------------------
fresh i
sed -i '/^actionlint|.*|\(env\|input\|install\)|/d' "$WORK/i/$SCRIPT_REL"
run "$WORK/i" "$WORK/i.up"
check "a tool declared with no pin site fails the run" 1 "$RC"
check "...saying which tool" 1 "$(printf '%s\n' "$ERR" | grep -c 'actionlint is declared as watched but no pin site reads it')"

fresh j
sed -i '/^PowerShell\/PSScriptAnalyzer\|^PSScriptAnalyzer|PowerShell/d' "$WORK/j/$SCRIPT_REL"
run "$WORK/j" "$WORK/j.up"
check "a pin site whose tool is not declared fails the run" 1 "$RC"
check "...saying which tool" 1 "$(printf '%s\n' "$ERR" | grep -c 'the PSScriptAnalyzer pin at .* has no upstream declared')"

# --- a pin in the workflows that no table lists --------------------------------
fresh k
{ echo "on:"; echo "  workflow_call:"; input new-tool-version 1.2.3 | sed 1d; } >"$WORK/k/.github/workflows/reusable-new.yml"
run "$WORK/k" "$WORK/k.up"
check "a new *-version input that no site reads fails the run" 1 "$RC"
check "...naming the file and the key" 1 "$(printf '%s\n' "$ERR" | grep -c 'reusable-new.yml has the pin new-tool-version')"

fresh l
echo '          cargo install brand-new-tool --version "=1.0.0" --locked' >>"$WORK/l/.github/workflows/release.yml"
run "$WORK/l" "$WORK/l.up"
check "a cargo install at an exact version of a tool no table names fails the run" 1 "$RC"

fresh m
# shellcheck disable=SC2016
echo '          cargo install cargo-deny --locked --version "$VERSION"' >>"$WORK/m/.github/workflows/release.yml"
echo "  OTHER_TOOL: 1" >>"$WORK/m/.github/workflows/release.yml"
run "$WORK/m" "$WORK/m.up"
check "an install whose version is a variable, and unrelated keys, are not mistaken for pins" 0 "$RC"

# --- an upstream that cannot be read --------------------------------------------
fresh n
sed -i 's|^rhysd/actionlint\tv1.7.12|rhysd/actionlint\tFAIL|' "$WORK/n.up"
run "$WORK/n" "$WORK/n.up"
check "a release that cannot be read fails the run" 1 "$RC"
check "...naming the repository" 1 "$(printf '%s\n' "$ERR" | grep -c 'could not read the latest release of rhysd/actionlint')"

fresh o
grep -v '^koalaman/shellcheck' "$WORK/o.up" >"$WORK/o.up2"
run "$WORK/o" "$WORK/o.up2"
check "a repository the stand-in does not know fails the run" 1 "$RC"

# --- the tables as they ship, against the real workflows --------------------------
real="$WORK/real"
mkdir -p "$real/.github/scripts/checks"
cp "$HERE/$SCRIPT_REL" "$real/$SCRIPT_REL"
cp -R "$HERE/.github/workflows" "$real/.github/workflows"
# Every release answers 999.0.0 in whatever form the tool tags it, so every pin
# reads as behind and the rows are the pins that were read.
awk -F'|' '/^[a-zA-Z0-9-]+\|[A-Za-z0-9-]+\/[A-Za-z0-9._-]+\|/ {print}' "$HERE/$SCRIPT_REL" >"$WORK/tools.txt"
: >"$WORK/real.up"
while IFS="|" read -r _ repo prefix; do
	printf '%s\t%s999.0.0\t%s999.0.0\n' "$repo" "$prefix" "$prefix" >>"$WORK/real.up"
done <"$WORK/tools.txt"
run "$real" "$WORK/real.up"
check "the shipped tables read every pin of the real workflows" 0 "$RC"
check "...all 14 sites, none skipped" 1 "$(grep -c 'watched 10 tools over 14 pin sites' "$WORK/err")"
check "...and every site comes out behind 999.0.0" 14 "$(printf '%s\n' "$OUT" | grep -c '999.0.0$')"

echo
echo "passed: $pass  failed: $fail"
echo "DONE pin-watch-tools.test.sh $pass $fail"
[ "$fail" -eq 0 ]
