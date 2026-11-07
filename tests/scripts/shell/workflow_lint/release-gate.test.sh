#!/usr/bin/env bash
#
# .github/scripts/checks/signing-key-isolation.py on the real release.yml, one
# change at a time. The three privileged jobs are pinned whole by hash, so every
# change to any field of them is refused, and the mutants below are generated
# from the workflow itself (every value of every field of sign, attest and
# release, every step removed, a field added) besides the named forms found in
# review. What is outside those jobs passes, unless it takes the key or a token.
#
# A fixed definition and a tripwire on it, not a proof about the release.
#
# Requires: python3 with PyYAML.
# shellcheck disable=SC2016
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
GATE="$HERE/.github/scripts/checks/signing-key-isolation.py"
REAL="$HERE/.github/workflows/release.yml"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
pass=0
fail=0
n=0

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

# <python body> -> status of the gate over release.yml after `spec` was changed.
run_gate() {
	n=$((n + 1))
	mkdir -p "$WORK/$n"
	python3 - "$REAL" "$WORK/$n/release.yml" "$1" <<'PY'
import sys, yaml
spec = yaml.safe_load(open(sys.argv[1]))
KEY = {"GLYNDOR_RELEASE_ED25519_KEY": "${{ secrets.GLYNDOR_RELEASE_ED25519_KEY }}"}
def step(job, prefix):
    return next(s for s in spec["jobs"][job]["steps"] if s.get("name", "").startswith(prefix))
def before(job, prefix, new):
    steps = spec["jobs"][job]["steps"]
    steps.insert(steps.index(step(job, prefix)), new)
exec(sys.argv[3])
open(sys.argv[2], "w").write(yaml.safe_dump(spec, sort_keys=False, width=1000))
PY
	python3 "$GATE" "$WORK/$n" >"$WORK/out" 2>&1
	echo $?
}
refused() { check "$1" 1 "$(run_gate "$2")"; }
accepted() { check "$1" 0 "$(run_gate "$2")"; }
gate_dir() { python3 "$GATE" "$1" >/dev/null 2>&1; echo $?; }

check "the shipped workflows pass" 0 "$(gate_dir "$HERE/.github/workflows")"
accepted "release.yml loaded and written back unchanged" "pass"
check "the hashes are reproducible" "$(grep -v '^#' "$HERE/.github/scripts/checks/release-pinned.txt")" "$(python3 "$GATE" --print-hashes "$HERE/.github/workflows")"
check "the failure says which job to review and how to regenerate" 1 "$(run_gate "step('sign', 'Sign binary')['name'] = 'x'" >/dev/null; grep -c 'job `sign` changed: review the whole diff.*--print-hashes' "$WORK/out")"

# --- every field of the three jobs, generated from the workflow ---------------------------
python3 - "$REAL" "$WORK" <<'PY'
import copy, sys, yaml
real = yaml.safe_load(open(sys.argv[1]))
out = sys.argv[2]
count = 0
def leaves(node, path=()):
    if isinstance(node, dict):
        for k, v in node.items():
            yield from leaves(v, path + (k,))
    elif isinstance(node, list):
        for i, v in enumerate(node):
            yield from leaves(v, path + (i,))
    else:
        yield path, node
def put(job, path, fn):
    global count
    spec = copy.deepcopy(real)
    node = spec["jobs"][job]
    for p in path[:-1]:
        node = node[p]
    node[path[-1]] = fn(node[path[-1]])
    count += 1
    yaml.safe_dump(spec, open(f"{out}/m{count}.yml", "w"), sort_keys=False, width=1000)
for job in ("sign", "attest", "release"):
    for path, value in leaves(real["jobs"][job]):
        put(job, path, lambda v: (not v) if isinstance(v, bool) else v + 1 if isinstance(v, int) else str(v) + "x")
    for i in range(len(real["jobs"][job]["steps"])):
        spec = copy.deepcopy(real)
        del spec["jobs"][job]["steps"][i]
        count += 1
        yaml.safe_dump(spec, open(f"{out}/m{count}.yml", "w"), sort_keys=False, width=1000)
    for field, value in (("if", "false"), ("continue-on-error", True), ("environment", "x"), ("outputs", {"a": "b"})):
        spec = copy.deepcopy(real)
        spec["jobs"][job][field] = value
        count += 1
        yaml.safe_dump(spec, open(f"{out}/m{count}.yml", "w"), sort_keys=False, width=1000)
    for s in range(len(real["jobs"][job]["steps"])):
        for field, value in (("if", "false"), ("continue-on-error", True), ("working-directory", "."), ("env", {"X": "1"})):
            spec = copy.deepcopy(real)
            spec["jobs"][job]["steps"][s][field] = value
            count += 1
            yaml.safe_dump(spec, open(f"{out}/m{count}.yml", "w"), sort_keys=False, width=1000)
print(count)
PY
survivors=0
total=0
for f in "$WORK"/m*.yml; do
	total=$((total + 1))
	d="$WORK/d$total"
	mkdir -p "$d"
	cp "$f" "$d/release.yml"
	[ "$(gate_dir "$d")" = 1 ] || { survivors=$((survivors + 1)); echo "SURVIVED $f"; }
done
check "no generated change of the three jobs passes ($total of them)" 0 "$survivors"
check "at least two hundred changes were generated" 1 "$([ "$total" -ge 200 ] && echo 1 || echo 0)"

# --- the forms found in review ---------------------------------------------------------------
for job_step in "sign:Sign binary" "attest:Check the artifacts" "release:Generate checksums"; do
	job="${job_step%%:*}"
	step="${job_step#*:}"
	refused "a build before the step, in $job" "before('$job', '$step', {'name': 'x', 'run': 'cargo build --release'})"
	refused "a helper outside the release folder, in $job" "before('$job', '$step', {'name': 'x', 'run': 'bash .github/scripts/build-and-inspect.sh'})"
	refused "a comment that ends in a backslash hiding a build, in $job" "before('$job', '$step', {'name': 'x', 'run': '# ignored \\\\\\\\\\ncargo build --release\\n'})"
	refused "another checkout with other inputs, in $job" "before('$job', '$step', {'uses': 'actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1', 'with': {'repository': 'other/repo', 'path': '.github'}})"
	refused "if: false on a step, in $job" "step('$job', '$step')['if'] = 'false'"
	refused "continue-on-error on a step, in $job" "step('$job', '$step')['continue-on-error'] = True"
	refused "a variable the pinned line runs, in $job" "step('$job', '$step')['env'] = {'venv_py': '\${{ runner.temp }}/artifacts/x'}"
done
refused "another download that escapes runner.temp through the matrix" "spec['jobs']['sign']['steps'].insert(1, {'uses': 'actions/download-artifact@9000827ccba6bdab643e8b6fd33ac0654aef8333', 'with': {'path': '\${{ matrix.dest }}'}})"
refused "the artifact copied over the signer by a relative path" "before('sign', 'Sign binary', {'name': 'x', 'working-directory': '.github/scripts/release', 'run': 'cp \"\$RUNNER_TEMP/artifacts/\$ASSET\" sign.py'})"
refused "a shell on the signing step" "step('sign', 'Sign binary')['shell'] = 'python {0}'"
refused "a default shell on the workflow" "spec['defaults'] = {'run': {'shell': 'python {0}'}}"
refused "BASH_ENV in the environment of the workflow" "spec['env'] = {'BASH_ENV': '/x'}"
refused "a different permission on the workflow" "spec['permissions'] = {'contents': 'write'}"

# --- who holds the key and who mints ----------------------------------------------------------
refused "the key on the Debian build step" "step('build-deb', 'Build the package')['env'] = KEY"
refused "toJSON(secrets) in a build" "step('build-deb', 'Build the package')['env'] = {'ALL': '\${{ toJSON(secrets) }}'}"
refused "a secret picked by an expression" "step('build-deb', 'Build the package')['env'] = {'K': '\${{ secrets[matrix.name] }}'}"
refused "tojson(secrets) in lower case" "step('build-deb', 'Build the package')['env'] = {'ALL': '\${{ tojson(secrets) }}'}"
refused "the key by an upper case context" "step('build-deb', 'Build the package')['env'] = {'K': '\${{ SECRETS.RELEASE_ED25519_KEY }}'}"
refused "the key by a lower case name" "step('build-deb', 'Build the package')['env'] = {'K': '\${{ secrets.release_ed25519_key }}'}"
refused "the key by a mixed case index" "step('build-deb', 'Build the package')['env'] = {'K': '\${{ Secrets[\"Release_Ed25519_Key\"] }}'}"
refused "secrets: inherit on a job" "spec['jobs']['build-deb']['secrets'] = 'inherit'"
refused "the key in the environment of the workflow" "spec['env'] = KEY"
refused "the signing step moved into the build job" "spec['jobs']['build']['steps'].append({'name': 'Sign', 'env': KEY, 'run': 'python3 .github/scripts/release/sign.py x'})"
refused "id-token on the build job" "spec['jobs']['build']['permissions']['id-token'] = 'write'"
refused "attestations: write on the build job" "spec['jobs']['build']['permissions']['attestations'] = 'write'"
refused "write-all on a job" "spec['jobs']['sbom']['permissions'] = 'write-all'"
refused "a reusable workflow called with the key" "spec['jobs']['sbom'] = {'uses': './.github/workflows/signer.yml', 'secrets': {'KEY': '\${{ secrets.GLYNDOR_RELEASE_ED25519_KEY }}'}}"
mkdir -p "$WORK/other"
printf 'jobs:\n  b:\n    runs-on: x\n    steps:\n      - run: echo\n        env:\n          K: ${{ secrets.GLYNDOR_RELEASE_ED25519_KEY }}\n' >"$WORK/other/other.yml"
check "the key in a job of another workflow" 1 "$(gate_dir "$WORK/other")"
printf 'jobs:\n  c:\n    uses: ./.github/workflows/x.yml\n    secrets: inherit\n' >"$WORK/other/other.yml"
check "secrets: inherit in a caller of another workflow" 1 "$(gate_dir "$WORK/other")"
printf 'permissions:\n  id-token: write\njobs:\n  deploy:\n    runs-on: x\n    steps:\n      - uses: some/oidc-action@v1\n' >"$WORK/other/other.yml"
check "a cloud deployment workflow with its own OIDC token is not the gate's business" 0 "$(gate_dir "$WORK/other")"
printf 'jobs:\n  b:\n    runs-on: x\n    steps:\n      - run: |\n          # secrets.GLYNDOR_RELEASE_ED25519_KEY is used only by release.yml\n          echo ok\n' >"$WORK/other/other.yml"
check "a comment in another workflow that names the key" 0 "$(gate_dir "$WORK/other")"

# --- what is outside the three jobs stays accepted -------------------------------------------
accepted "a build step with its own variables in the build job" "spec['jobs']['build']['steps'].append({'name': 'x', 'env': {'PATH': 'x'}, 'run': 'cargo build --release'})"
accepted "a change in the Debian build job" "spec['jobs']['build-deb']['timeout-minutes'] = 99"
accepted "a change in the SBOM job" "spec['jobs']['sbom']['steps'].append({'name': 'x', 'run': 'echo hi'})"
accepted "a defaults block on a job that is not pinned" "spec['jobs']['build']['defaults'] = {'run': {'shell': 'bash'}}"

echo
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
