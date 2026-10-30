#!/usr/bin/env bash
# Exercise the classifier and the actual workflow decision with a fake git.
set -uo pipefail
export LC_ALL=C
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
script="$HERE/.github/scripts/checks/binary-budget-paths.sh"
workflow="$HERE/.github/workflows/binary-budget.yml"
pass=0
fail=0
check() {
	if [ "$2" = "$3" ]; then
		echo "ok    $1"
		pass=$((pass + 1))
	else
		echo "FAIL  $1 (expected $2, got $3)"
		fail=$((fail + 1))
	fi
}
# Keep the evidence behind every exemption executable. A production parent
# losing cfg(test) must not leave a permanent exemption behind in the judge.
python3 - "$HERE" "$WORK/paths" <<'PY'
from pathlib import Path
import re
import sys
root = Path(sys.argv[1])
pairs = {
    "objects/secrets/create.rs": "create_tests.rs",
    "host_files/copy/pack/transport.rs": "transport_tests.rs",
    "host_files/watch/sync.rs": "sync_filter_tests.rs",
    "observe/stats/mod.rs": "stats_tests.rs",
    "host_files/watch/filtering/events.rs": "events_tests.rs",
}
paths = []
for parent, name in pairs.items():
    parent = root / "internal/backends/engine" / parent
    text = parent.read_text()
    declaration = r'#\[cfg\(test\)\]\s*#\[path = "' + re.escape(name) + r'"\]\s*mod \w+;'
    assert re.search(declaration, text), f"missing test-only declaration: {parent}"
    target = parent.parent / name
    assert target.is_file()
    # Reject a second textual path reference to the same file, including
    # include_str!/include_bytes!, which would make its bytes a build input.
    references = []
    for source in (root / "internal").rglob("*.rs"):
        for value in re.findall(r'"([^"\n]+)"', source.read_text()):
            if name in value and (source.parent / value).resolve() == target.resolve():
                references.append(source)
    assert references == [parent], f"additional consumers of {target}: {references}"
    paths.append(str(target.relative_to(root)))
Path(sys.argv[2]).write_text("\n".join(paths) + "\n")
PY
rc=$?
check "all exemptions still have exclusive cfg(test) declarations" 0 "$rc"
run_case() {
	local rc=0 out
	out="$(bash "$script" "$2" 2>&1)" || rc=$?
	check "classify $2" "$1" "$rc"
	if [ "$1" = 1 ]; then
		check "refusal explains build" 1 "$(printf '%s' "$out" | grep -cF 'release build required:')"
	fi
}
while IFS= read -r path; do run_case 0 "$path"; done < "$WORK/paths"
paths="$(cat "$WORK/paths")"
run_case 0 "$paths"
for path in internal/future_tests.rs internal/backends/engine/objects/secrets/create.rs \
	internal/backends/engine/host_files/copy/pack/transport.rs \
	internal/backends/engine/host_files/watch/sync.rs \
	internal/backends/engine/observe/stats/mod.rs \
	internal/backends/engine/host_files/watch/filtering/events.rs \
	Cargo.toml Cargo.lock build.rs .cargo/config.toml rust-toolchain.toml \
	bench/budgets/binary-budget-mib .github/workflows/binary-budget.yml \
	.github/scripts/checks/binary-budget-paths.sh README.md '"internal/future_tests.rs"'; do
	run_case 1 "$path"
	run_case 1 "$paths
$path"
done
run_case 1 ''
run_case 1 "$paths
"
rc=0
bash "$script" > "$WORK/log" 2>&1 || rc=$?
check "missing argument builds" 1 "$rc"

python3 - "$workflow" "$WORK/decide.sh" <<'PY'
import sys
import yaml
spec = yaml.safe_load(open(sys.argv[1]))
on = spec.get("on", spec.get(True))
for event in ("push", "pull_request"):
    paths = on[event]["paths"]
    for required in ("internal/**", "Cargo.toml", "Cargo.lock", ".cargo/**", "build.rs",
                     "rust-toolchain*", "bench/budgets/binary-budget-mib",
                     ".github/workflows/binary-budget.yml",
                     ".github/scripts/checks/binary-budget-paths.sh"):
        assert required in paths, f"{event} missing input: {required}"
job = spec["jobs"]["size"]
assert job["name"] == "Release binary fits its budget"
steps = job["steps"]
checkout = next(s for s in steps if s.get("uses", "").startswith("actions/checkout@"))
assert checkout["with"]["fetch-depth"] == 0
for name in ("Build the release binary", "Compare against the budget"):
    step = next(s for s in steps if s.get("name") == name)
    assert step["if"] == "steps.inputs.outputs.build == 'true'", name
open(sys.argv[2], "w").write(next(s["run"] for s in steps if s.get("id") == "inputs"))
PY
rc=$?
check "workflow keeps build inputs, check name and both step conditions" 0 "$rc"
mkdir "$WORK/bin"
cat > "$WORK/bin/git" <<'GIT'
#!/usr/bin/env bash
case "$1" in
  diff)
    [ "$*" = 'diff --name-only --no-renames base head' ] || exit 2
    [ "${DIFF_ERROR:-0}" = 0 ] || exit 1
    printf '%s\n' "$CHANGED"
    ;;
  show)
    [ "$2" = base:.github/scripts/checks/binary-budget-paths.sh ] || exit 2
    [ "${MISSING_JUDGE:-0}" = 0 ] || exit 1
    cat "$JUDGE"
    ;;
  *) exit 2 ;;
esac
GIT
chmod +x "$WORK/bin/git"
export JUDGE="$script"
decide() {
	local rc=0
	: > "$WORK/output"
	PATH="$WORK/bin:$PATH" EVENT="$1" BASE_SHA=base HEAD_SHA=head CHANGED="$2" \
		GITHUB_OUTPUT="$WORK/output" bash "$WORK/decide.sh" > "$WORK/log" 2>&1 || rc=$?
	check "decide $1 exits successfully" 0 "$rc"
	check "decide $1 $2" "build=$3" "$(cat "$WORK/output")"
}
decide pull_request "$paths" false
decide push "$paths" false
decide pull_request "$paths
Cargo.toml" true
decide pull_request '' true
DIFF_ERROR=1 decide pull_request "$paths" true
MISSING_JUDGE=1 decide push "$paths" true
decide workflow_dispatch "$paths" true
# The checked-out judge is irrelevant: only git show base supplies its code.
printf 'exit 1\n' > "$WORK/base-judge"
JUDGE="$WORK/base-judge" decide pull_request "$paths" true
printf 'exit 0\n' > "$WORK/base-judge"
JUDGE="$WORK/base-judge" decide pull_request unknown false
printf 'invalid shell (\n' > "$WORK/base-judge"
JUDGE="$WORK/base-judge" decide pull_request "$paths" true

echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
