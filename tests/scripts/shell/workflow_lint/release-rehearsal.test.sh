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
# The tag is also authorized before it runs: the verification asks whether the tagged commit is on
# `main` in commands of its own workflow, before it runs one line of a file of that commit (which, if it
# is not on `main`, is nobody's code that was reviewed). That is held by running the steps of the
# verification in order on a commit that is not on `main`, whose `check-tag.sh` plants a `git` that
# answers yes to the question, and seeing that the question is asked first and that script never runs.
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
        after = [x for x in steps[checkout + 1:] if x.get("name") not in ("Trust the workspace", "Tag must be on main")]
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

echo "== the tag is on main before a line of it runs"
# The steps of `verify` as the runner runs them: in order, each with its env (the few expressions of
# this job read from what the case says), a path that grows by what a step writes to GITHUB_PATH, and
# the stop at the first step that fails (a step that may fail does not stop it). The checkout is the fixture's.
emulate() { # <workflow file> <work dir> <tag>
	python3 -I - "$@" <<'EOF'
import os, re, subprocess, sys
sys.path.insert(0, os.environ["LIB"])
import ci_yaml  # noqa: E402

path, work, tag = sys.argv[1:4]
spec = ci_yaml.load(path)
steps = spec["jobs"]["verify"]["steps"]
path_file, out_file = os.path.join(work, "gh_path"), os.path.join(work, "gh_out")
for f in (path_file, out_file):
    open(f, "w").close()
extra = []
SUBST = {"inputs.tag": tag, "github.token": "token", "github.repository": "o/r", "inputs.dry_run && 'rehearsal' || 'release'": "release"}

def expand(v):
    return re.sub(r"\$\{\{\s*(.*?)\s*\}\}", lambda m: SUBST.get(m.group(1), ""), str(v))

for st in steps:
    if "run" not in st:
        continue
    cond = st.get("if")
    if cond is not None and expand(cond) != "true":
        continue
    env = dict(os.environ, GITHUB_PATH=path_file, GITHUB_OUTPUT=out_file, PODUP_PUBLISHED_TAGS=os.path.join(work, "published"))
    env["PATH"] = ":".join(extra + [env["PATH"]])
    env.update({k: expand(v) for k, v in (st.get("env") or {}).items()})
    done = subprocess.run(["bash", "-e", "-c", st["run"]], cwd=os.path.join(work, "checkout"), env=env, capture_output=True, text=True)
    extra = [l.strip() for l in open(path_file) if l.strip()][::-1]
    if done.returncode != 0 and not st.get("continue-on-error"):
        print(f"stopped at: {st.get('name')}")
        sys.exit(1)
print("ran every step")
EOF
}

# A repository with `main`, and a tag that is on it or is not. The tag that is not carries a
# `check-tag.sh` that leaves a mark when it runs and puts a `git` of its own on the path, one that
# answers yes to `merge-base --is-ancestor` and is the real git for everything else.
fixture() { # <name> <on-main|off-main> -> the work directory
	local w="$WORK/$1" origin="$WORK/$1/origin" g="git -c user.name=t -c user.email=t@example.invalid -c commit.gpgsign=false -c tag.gpgsign=false"
	mkdir -p "$w" && git init -q -b main "$origin"
	( cd "$origin" && mkdir -p .github/scripts/release && echo main > f && $g add -A && $g commit -q -m main
	  if [ "$2" = off-main ]; then
		git checkout -q -b elsewhere
		cat > .github/scripts/release/check-tag.sh <<SH
#!/usr/bin/env bash
echo ran > "$w/mark"
mkdir -p "$w/fake" && printf '#!/usr/bin/env bash\n[ "\$1 \$2" = "merge-base --is-ancestor" ] && exit 0\nexec $(command -v git) "\$@"\n' > "$w/fake/git" && chmod +x "$w/fake/git"
echo "$w/fake" >> "\$GITHUB_PATH"
SH
		echo 'pass' > .github/scripts/release/check-key-slots.py
		echo ':' > .github/scripts/release/check-unpublished.sh
		echo x >> f
		$g add -A && $g commit -q -m other
	  fi
	  $g tag -a v9.9.9 -m v9.9.9 )
	printf 'v1.0.0\n' > "$w/published"
	git init -q "$w/checkout"
	( cd "$w/checkout" && git remote add origin "file://$origin" && git fetch -q --depth 1 origin "+refs/tags/v9.9.9:refs/tags/v9.9.9" && git checkout -q --detach refs/tags/v9.9.9 )
	echo "$w"
}
export LIB="$HERE/.github/scripts/lib"
VWF="$HERE/.github/workflows/reusable-release-verify.yml"
ran() { # <workflow> <case name> <on-main|off-main> -> "<did the question stop the run>|<did a script of the tag run>"
	local w out
	w="$(fixture "$2" "$3")"
	out="$(emulate "$1" "$w" v9.9.9 2>&1 | tail -1)"
	echo "$([ "$out" = "stopped at: Tag must be on main" ] && echo stopped || echo went-on)|$([ -e "$w/mark" ] && echo script-ran || echo script-did-not-run)"
}
check "a tag on main goes through the question" "went-on|script-did-not-run" "$(ran "$VWF" a on-main)"
check "a tag that is not on main is stopped by the question, and its script never runs" "stopped|script-did-not-run" "$(ran "$VWF" b off-main)"

plantv() { # <description> <python expression on the text s> <what the off-main case then shows>
	local f="$WORK/plantv-$RANDOM.yml"
	cp "$VWF" "$f"
	if python3 - "$f" "$2" <<'EOF'
import sys
p, expr = sys.argv[1], sys.argv[2]
s = open(p).read()
t = eval(expr)
open(p, "w").write(t)
sys.exit(0 if t != s else 3)
EOF
	then
		check "plant: $1" "$3" "$(ran "$f" "p$RANDOM" off-main)"
	else
		check "the plant applies: $1" applied not-applied
	fi
}
# the question after the scripts of the tag, as it was
plantv "the question asked after the scripts of the tag" "(lambda b: s.replace(b, '').replace('      # Before anything is installed:', b + '      # Before anything is installed:', 1))(s[s.index('      # The first thing this job does with the tag'):s.index('      - name: Take the release scripts')])" "went-on|script-ran"
plantv "the question not asked" "(lambda b: s.replace(b, ''))(s[s.index('      # The first thing this job does with the tag'):s.index('      - name: Take the release scripts')])" "went-on|script-ran"
plantv "the question that may fail" "s.replace('      - name: Tag must be on main\n', '      - name: Tag must be on main\n        continue-on-error: true\n', 1)" "went-on|script-ran"
plantv "the question asked only in a rehearsal" "s.replace('      - name: Tag must be on main\n', '      - name: Tag must be on main\n        if: \${{ inputs.dry_run }}\n', 1)" "went-on|script-ran"
plantv "the question of a script of the tag" "s.replace('          git fetch --unshallow --no-tags origin \"+refs/heads/main:refs/remotes/origin/main\" 2>/dev/null \\\\\n            || git fetch --no-tags origin \"+refs/heads/main:refs/remotes/origin/main\"\n          if ! git merge-base --is-ancestor HEAD refs/remotes/origin/main; then', '          if ! bash .github/scripts/release/check-tag.sh v9.9.9; then', 1)" "went-on|script-ran"

echo
echo "passed: $pass  failed: $fail"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
