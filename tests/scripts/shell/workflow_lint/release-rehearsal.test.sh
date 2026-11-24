#!/usr/bin/env bash
#
# A rehearsal of the release may name a tag that is already published, and a real release
# may not. Both directions are held here, on the script that decides and on the workflows
# that call it.
#
#   check-unpublished.sh   a published tag stops a release and not a rehearsal; a mode that is
#                          neither word stops, so a typo cannot turn the refusal off;
#   the workflows          the mode is read from `dry_run` and nothing else, `dry_run` reaches
#                          the verification and the builds, every checkout of a job that runs
#                          a release script is followed by the step that takes the scripts
#                          from the commit that started a rehearsal (the tag may be older than
#                          them), and the jobs that hold the key are in the environment
#                          `release` in a rehearsal as in a release: a rehearsal is started from
#                          `main`, which the environment admits, and nothing weakens that.
#
# Each plant breaks one of those in a copy of the workflows and expects the judge to say so.
#
# Requires: python3 with PyYAML.
set -u
export LC_ALL=C

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
SCRIPT="$HERE/.github/scripts/release/check-unpublished.sh"
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

printf 'v5.20.0\nv6.0.0\n' >"$WORK/published"
run() { # <tag> <mode> -> "<status>|<output>"
	local rc=0 out
	out="$(PODUP_PUBLISHED_TAGS="$WORK/published" bash "$SCRIPT" "$@" 2>&1)" || rc=$?
	echo "$rc|$out"
}
status() { run "$@" | cut -d'|' -f1; }

echo "== the script"
check "a real release of a published tag is refused" 1 "$(status v6.0.0 release)"
check "...and says why" 1 "$(run v6.0.0 release | grep -c 'is already published; refusing to rebuild or publish it again')"
check "a rehearsal of a published tag goes on" 0 "$(status v6.0.0 rehearsal)"
check "...and says it publishes nothing" 1 "$(run v6.0.0 rehearsal | grep -c 'a rehearsal publishes nothing')"
check "a real release of a tag nobody published goes on" 0 "$(status v6.0.1 release)"
check "a rehearsal of a tag nobody published goes on" 0 "$(status v6.0.1 rehearsal)"
check "a tag that only starts like a published one is not published" 0 "$(status v6.0 release)"
check "a tag that contains a published one is not published" 0 "$(status v6.0.00 release)"
check "an empty list publishes nothing" 0 "$(: >"$WORK/published" && status v6.0.0 release)"
printf 'v5.20.0\nv6.0.0\n' >"$WORK/published"
check "a mode that is neither word is refused" 2 "$(status v6.0.0 dry_run)"
check "...also for a tag nobody published" 2 "$(status v6.0.1 '')"
check "a missing mode is refused" 2 "$(status v6.0.0)"
check "no argument is refused" 2 "$(status)"

# The judge: the rules of the workflows. One line per problem; empty when they hold.
judge() { # <workflow directory>
	python3 -I - "$1" "$HERE/.github/scripts/lib" <<'EOF'
import os
import sys

sys.path.insert(0, sys.argv[2])
import ci_yaml  # noqa: E402

root = sys.argv[1]
rel = ci_yaml.load(os.path.join(root, "release.yml"))
ver = ci_yaml.load(os.path.join(root, "reusable-release-verify.yml"))
bld = ci_yaml.load(os.path.join(root, "reusable-release-build.yml"))
SWITCH = "${{ github.event_name == 'workflow_dispatch' && inputs.dry_run }}"
OVERLAY = "Take the release scripts from the commit that started the rehearsal"

for name, spec in (("verify", ver), ("build", bld)):
    declared = ((spec.get("on") or spec.get(True) or {}).get("workflow_call") or {}).get("inputs", {}).get("dry_run")
    if not declared or declared.get("type") != "boolean" or declared.get("default") is not False:
        print(f"reusable-release-{name}.yml: no boolean input dry_run that is off by default")
for job, who in (("verify", "verification"), ("builds", "builds")):
    if (rel["jobs"][job].get("with") or {}).get("dry_run") != SWITCH:
        print(f"release.yml: the {who} does not receive dry_run from the dispatch")

v = next((s for s in ver["jobs"]["verify"]["steps"] if s.get("name") == "Refuse an already published release"), None)
if v is None:
    print("the verification has no step that refuses a published release")
else:
    if (v.get("env") or {}).get("MODE") != "${{ inputs.dry_run && 'rehearsal' || 'release' }}":
        print("the mode of the published-release check is not read from dry_run")
    if v.get("run", "").strip() != 'bash .github/scripts/release/check-unpublished.sh "$TAG" "$MODE"':
        print("the published-release check is not the script with the tag and the mode")
    if v.get("if") is not None or v.get("continue-on-error"):
        print("the published-release check is conditional or may fail")

canon = None
FOLDERS = (".github/scripts/release", ".github/scripts/packaging")
for path, spec, skip in (("reusable-release-verify.yml", ver, ()), ("reusable-release-build.yml", bld, ()), ("release.yml", rel, ("attest",))):
    for job, body in spec["jobs"].items():
        steps = body.get("steps") or []
        checkout = next((i for i, s in enumerate(steps) if str(s.get("uses", "")).startswith("actions/checkout@")), None)
        if checkout is None or job in skip:
            continue
        # A job takes the scripts of a rehearsal when it runs one of the two folders that are new, and
        # only then: the rest of the tag, the tests of its Debian build included, stays as the tag has it.
        runs = " ".join(str(s.get("run", "")) for s in steps if s.get("name") != OVERLAY)
        needs = any(f in runs for f in FOLDERS)
        # A container job trusts its workspace first, so that git works at all; the scripts follow.
        after = [x for x in steps[checkout + 1:] if x.get("name") != "Trust the workspace"]
        nxt = after[0] if after else {}
        if needs and nxt.get("name") != OVERLAY:
            print(f"{path}: job `{job}` runs a script of the release and does not take the scripts of a rehearsal right after its checkout")
            continue
        if not needs:
            if any(s.get("name") == OVERLAY for s in steps):
                print(f"{path}: job `{job}` runs no script of the release and takes them anyway, over the tag's own tests")
            continue
        if nxt.get("if") != "${{ inputs.dry_run }}":
            print(f"{path}: job `{job}`: the step that takes the scripts is not only for a rehearsal")
        canon = canon or nxt.get("run")
        if nxt.get("run") != canon:
            print(f"{path}: job `{job}`: the step that takes the scripts differs from the others")
if canon is None:
    print("no job takes the scripts of a rehearsal")
else:
    for needle in ('"$GITHUB_SHA"', "skip-worktree", ".git/info/exclude", ".github/scripts/release", ".github/scripts/packaging"):
        if needle not in canon:
            print(f"the step that takes the scripts lacks {needle}")
    if "git archive FETCH_HEAD .github/scripts |" in canon or "rm -rf .github/scripts " in canon:
        print("the step that takes the scripts replaces the whole scripts folder, and the tag's own tests read it")

for job in ("sign", "sign-packages", "seal"):
    if rel["jobs"][job].get("environment") != "release":
        print(f"release.yml: job `{job}` is not in the environment `release` in every run, a rehearsal included")
for job in ("release", "attest"):
    if rel["jobs"][job].get("environment") not in (None, ""):
        print(f"release.yml: job `{job}` has an environment")

audit = next((s for s in ver["jobs"]["audit"]["steps"] if s.get("name", "").startswith("Audit dependencies")), {})
if audit.get("continue-on-error") != "${{ inputs.dry_run }}":
    print("the audit does not stop a release and go past in a rehearsal")
EOF
}

echo "== the workflows"
check "the real workflows follow the rules" "" "$(judge "$HERE/.github/workflows")"

plant() { # <description> <file> <python expression on the text s> <fragment of the expected line>
	local dir="$WORK/plant-$RANDOM"
	mkdir -p "$dir"
	cp "$HERE"/.github/workflows/release.yml "$HERE"/.github/workflows/reusable-release-*.yml "$dir/"
	if python3 - "$dir/$2" "$3" <<'EOF'
import sys

p, expr = sys.argv[1], sys.argv[2]
s = open(p).read()
# The expression is written in this test file, never taken from input: it builds a planted copy.
t = eval(expr)
open(p, "w").write(t)
sys.exit(0 if t != s else 3)
EOF
	then
		:
	else
		check "the plant applies: $1" applied not-applied
		return
	fi
	check "plant: $1" 1 "$(judge "$dir" | grep -cF "$4")"
}
V=reusable-release-verify.yml
B=reusable-release-build.yml
R=release.yml
plant "the mode always rehearsal" $V "s.replace(\"inputs.dry_run && 'rehearsal' || 'release'\", \"'rehearsal'\")" "not read from dry_run"
plant "the mode always release" $V "s.replace(\"inputs.dry_run && 'rehearsal' || 'release'\", \"'release'\")" "not read from dry_run"
plant "the check back inline, on every mode" $V "s.replace('bash .github/scripts/release/check-unpublished.sh \"\$TAG\" \"\$MODE\"', 'gh api repos/x/releases')" "is not the script"
plant "the check skipped in a rehearsal by a condition" $V "s.replace('      - name: Refuse an already published release\n', '      - name: Refuse an already published release\n        if: \${{ !inputs.dry_run }}\n')" "conditional or may fail"
plant "the verification not told about the rehearsal" $R "s.replace(\"      tag: \${{ github.event_name == 'workflow_dispatch' && inputs.tag || github.ref_name }}\n      dry_run: \${{ github.event_name == 'workflow_dispatch' && inputs.dry_run }}\n\", \"      tag: \${{ github.event_name == 'workflow_dispatch' && inputs.tag || github.ref_name }}\n\")" "the verification does not receive dry_run"
plant "the builds not told about the rehearsal" $R "s.replace(\"      tag: \${{ needs.verify.outputs.tag }}\n      dry_run: \${{ github.event_name == 'workflow_dispatch' && inputs.dry_run }}\n\", \"      tag: \${{ needs.verify.outputs.tag }}\n\")" "the builds does not receive dry_run"
plant "the input of the verification on by default" $V "s.replace('        default: false\n        type: boolean', '        default: true\n        type: boolean', 1)" "no boolean input dry_run"
plant "the scripts of a rehearsal taken in every run" $B "s.replace('        if: \${{ inputs.dry_run }}\n        run: |\n          set -o pipefail', '        run: |\n          set -o pipefail', 1)" "not only for a rehearsal"
plant "a checkout without the step that takes the scripts" $B "s.replace('      - name: Take the release scripts from the commit that started the rehearsal\n', '      - name: Take nothing\n', 1)" "runs a script of the release and does not take the scripts of a rehearsal right after its checkout"
plant "the whole scripts folder taken in one job" $R "s.replace('rm -rf .github/scripts/release .github/scripts/packaging &&', 'rm -rf .github/scripts &&', 1)" "differs from the others"
plant "the scripts taken in the Debian job, over the tests of the tag" $B "(lambda b: s.replace('      - name: Trust the workspace\n', b + '      - name: Trust the workspace\n', 1))(s[s.index('      - name: Take the release scripts'):s.index('      - name:', s.index('      - name: Take the release scripts') + 20)])" "takes them anyway, over the tag's own tests"
plant "the scripts left in the worktree as changed files" $R "s.replace('skip-worktree', 'no-skip-worktree', 1)" "differs from the others"
plant "a key job without its environment in a rehearsal" $R "s.replace('    environment: release\n', '    environment: \${{ !inputs.dry_run && \\'release\\' || \\'\\' }}\n', 1)" "is not in the environment"
plant "a key job in another environment" $R "s.replace('    environment: release\n', '    environment: elsewhere\n', 1)" "is not in the environment"
plant "the publishing job given an environment" $R "s.replace('  release:\n    name:', '  release:\n    environment: release\n    name:', 1)" "job \`release\` has an environment"
plant "the audit stopping a rehearsal" $V "s.replace('continue-on-error: \${{ inputs.dry_run }}', 'continue-on-error: false')" "does not stop a release and go past"

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
