#!/usr/bin/env bash
#
# A workflow that runs when release.yml changes also runs when a reusable workflow that release.yml
# calls changes. The builds, the verification, the rehearsal and what they build moved out of release.yml into
# reusables, and a filter that still names only release.yml stops seeing every change to what it was
# written to watch. The set of reusables is read from release.yml (`uses: ./.github/workflows/...`), so
# a new one is a new requirement here, and the filters are read from the workflows, on both the push and
# the pull request lists.
#
# Requires: python3 with PyYAML.
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
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

# <root> -> "<workflow> <event>: <missing path>" for every filter that names release.yml and leaves out a
# reusable it calls.
gaps() {
	python3 - "$1" <<'PY'
import glob, os, sys, yaml
root = sys.argv[1]
wf = os.path.join(root, ".github", "workflows")
release = yaml.safe_load(open(os.path.join(wf, "release.yml")))
called = sorted(
    j["uses"][2:] for j in release["jobs"].values()
    if str(j.get("uses", "")).startswith("./.github/workflows/")
)
print("called:", " ".join(called), file=sys.stderr)
if not called:
    print("release.yml calls no reusable workflow")
for path in sorted(glob.glob(os.path.join(wf, "*.yml"))):
    spec = yaml.safe_load(open(path))
    on = spec.get("on", spec.get(True)) or {}
    if not isinstance(on, dict):
        continue
    for event in ("push", "pull_request"):
        paths = (on.get(event) or {}).get("paths") if isinstance(on.get(event), dict) else None
        if not paths or ".github/workflows/release.yml" not in paths:
            continue
        for need in called:
            if need not in paths:
                print(f"{os.path.basename(path)} {event}: {need}")
PY
}

check "release.yml calls the verification, the builds and the rehearsal" "called: .github/workflows/reusable-release-build.yml .github/workflows/reusable-release-rehearsal.yml .github/workflows/reusable-release-verify.yml" "$(gaps "$HERE" 2>&1 >/dev/null)"
check "no filter that names release.yml leaves out a reusable it calls" "" "$(gaps "$HERE" 2>/dev/null)"
check "several workflows filter by release.yml, so the check looks at something" 1 "$([ "$(grep -l 'workflows/release.yml' "$HERE"/.github/workflows/*.yml | wc -l)" -ge 5 ] && echo 1 || echo 0)"

fresh() { rm -rf "$WORK/t"; mkdir -p "$WORK/t/.github"; cp -r "$HERE/.github/workflows" "$WORK/t/.github/workflows"; }
for w in debian-build linux-hardening windows-arm64 asset-contract lint-shell; do
	for r in reusable-release-build reusable-release-rehearsal reusable-release-verify; do
		fresh
		python3 - "$WORK/t/.github/workflows/$w.yml" "$r" <<'PY'
import sys
path, r = sys.argv[1], sys.argv[2]
lines = open(path).read().split("\n")
out, done = [], False
for line in lines:
    if line.strip() == f'- ".github/workflows/{r}.yml"' and not done:
        done = True
        continue
    out.append(line)
assert done, "nothing to take out"
open(path, "w").write("\n".join(out))
PY
		check "$w without $r in a list is found" 1 "$(gaps "$WORK/t" 2>/dev/null | grep -c "^$w.yml .*: .github/workflows/$r.yml")"
	done
done
fresh
python3 - "$WORK/t/.github/workflows/release.yml" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read()
s = s.replace("    uses: ./.github/workflows/reusable-release-build.yml\n", "    uses: ./.github/workflows/reusable-release-build.yml\n  more:\n    uses: ./.github/workflows/reusable-brand-new.yml\n", 1)
open(p, "w").write(s)
PY
check "a reusable that release.yml starts to call is a new requirement" 1 "$([ "$(gaps "$WORK/t" 2>/dev/null | grep -c 'reusable-brand-new.yml')" -ge 5 ] && echo 1 || echo 0)"

echo
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
