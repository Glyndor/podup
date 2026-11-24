#!/usr/bin/env python3
"""The sabotages the wiring snapshot must catch, as data.

Each entry is (description, workflow file, text to find once, text to put there).
Text is literal, so there is no quoting to get wrong. `sabotage.py --count`
prints how many there are, `sabotage.py --describe N` the description of one,
and `sabotage.py N TREE` applies the N-th to the copy of the workflows in TREE
and fails if the text it expects is not there (a plant that changes nothing
proves nothing).
"""
import os
import sys

R = "reusable-rust-ci.yml"
IF_TEST = "    if: ${{ !cancelled() && (github.event_name != 'pull_request' || needs.lint.result != 'failure') && (needs.classify.result != 'success' || needs.classify.outputs.run != 'false') }}\n    runs-on: ubuntu-latest\n    # Bound: worst of the last three successful runs on 2026-09-02 was 106s"

PLANTS = [
    # 1. The run: of two suites crossed.
    ("the command of the Linux tests swapped for another suite's", R,
     "run: cargo test --locked --all-features --workspace", "run: cargo test --locked --all-features --lib"),
    ("the coverage command swapped for another suite's", R,
     "cargo llvm-cov --locked --workspace", "cargo llvm-cov --locked --lib"),
    # 2. The if of two suites crossed.
    ("the matrix given the coverage's switch", R,
     "if: ${{ inputs.extra-test-os != '[]' &&", "if: ${{ inputs.coverage-threshold > 0 &&"),
    # 3. A suite handed a fixed false.
    ("a fixed false on the Linux tests", R, IF_TEST, IF_TEST.replace(
        "if: ${{ !cancelled() && (github.event_name != 'pull_request' || needs.lint.result != 'failure') && (needs.classify.result != 'success' || needs.classify.outputs.run != 'false') }}", "if: false")),
    ("a fixed false on the Rust pipeline in its caller", "ci.yml", "  rust:\n", "  rust:\n    if: false\n"),
    ("a duplicate `if` on the Linux tests, which PyYAML's safe_load reads as the last one", R,
     "  test:\n    name: Test\n", "  test:\n    name: Test\n    if: true\n"),
    ("a duplicate key in the gate", R, "  gate:\n", "  gate:\n    runs-on: ubuntu-latest\n"),
    ("the cancel job running the working tree's script instead of the base copy", "reusable-cancel-others.yml",
     'bash "$RUNNER_TEMP/others.sh"', "bash .github/scripts/ci/cancel-others.sh"),
    ("the layers no longer waiting for lint", R, "needs: [classify, lint]\n    if: ${{ !cancelled() && (github.event_name != 'pull_request' || needs.lint.result != 'failure') && (needs.classify.result != 'success' || needs.classify.outputs.run != 'false') }}", "needs: [classify]\n    if: ${{ !cancelled() && (needs.classify.result != 'success' || needs.classify.outputs.run != 'false') }}"),
    # The contract holds everything that changes how a job runs, by equality.
    ("the matrix's default shell turned into one that exits 0", R, "        shell: bash\n        working-directory: ${{ inputs.working-directory }}\n    steps:\n      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1\n        with:\n          persist-credentials: false\n\n      - name: Install toolchain", "        shell: \"bash -c 'exit 0' {0}\"\n        working-directory: ${{ inputs.working-directory }}\n    steps:\n      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1\n        with:\n          persist-credentials: false\n\n      - name: Install toolchain"),
    ("a default shell added to the whole workflow", "ci.yml", "permissions:\n  contents: read\n", "permissions:\n  contents: read\n\ndefaults:\n  run:\n    shell: \"bash -c 'exit 0' {0}\"\n"),
    ("a step that runs its script through a shell that exits 0", R, "      - name: Tests\n        run: cargo test --locked --all-features --workspace", "      - name: Tests\n        shell: \"bash -c 'exit 0' {0}\"\n        run: cargo test --locked --all-features --workspace"),
    ("a suite that may fail and still count", R, "  test:\n    name: Test\n", "  test:\n    name: Test\n    continue-on-error: true\n"),
    ("the gate that may fail and still count", R, "  gate:\n", "  gate:\n    continue-on-error: true\n"),
    ("a step of the gate that may fail and still count", R, "      - name: Certify the plan and every skip\n", "      - name: Certify the plan and every skip\n        continue-on-error: true\n"),
    ("a timeout that cuts a suite short", R, "    timeout-minutes: 20\n    defaults:", "    timeout-minutes: 1\n    defaults:"),
    ("a container that runs a job in another image", "reusable-line-limit.yml", "  line-limit:\n", "  line-limit:\n    container: alpine\n"),
    ("a property nobody listed on a job", "reusable-dco.yml", "jobs:\n  dco:\n", "jobs:\n  dco:\n    environment: elsewhere\n"),
    # The push to develop repeats nothing the queue ran.
    ("CI running again on a push to develop", "ci.yml", "  push:\n    branches: [main]\n", "  push:\n    branches: [main, develop]\n"),
    ("the audit running again on a push to develop", "audit.yml", "  merge_group:\n    types: [checks_requested]\n", "  merge_group:\n    types: [checks_requested]\n  push:\n    branches:\n      - develop\n"),
    ("the line limit running again on a push to develop", "line-limit.yml", "  push:\n    branches:\n      - main\n", "  push:\n    branches:\n      - main\n      - develop\n"),
    # 4. The plan variables of two suites crossed in the gate.
    ("the run and build answers crossed in the gate", R, "RUN=$(out classify run)", "RUN=$(out classify build)"),
    ("the switches of two suites crossed in the gate", R, 'judge msrv "$MSRV_ON"', 'judge msrv "$DOC_ON"'),
    ("the docs and CI-only answers crossed in the gate", R, "DOCS=$(out classify docs_only)", "DOCS=$(out classify ci_only)"),
    # 5. The gate reading the result of another job for a suite.
    ("the gate reading the tests' result for the coverage", R, "judge coverage", "judge test"),
    ("a job left out of the gate's needs", R,
     "needs: [classify, lint, test, test-extra, coverage, msrv, doc, package, semver]",
     "needs: [classify, lint, test, test-extra, coverage, msrv, package, semver]"),
    ("the gate losing always()", R,
     "needs: [classify, lint, test, test-extra, coverage, msrv, doc, package, semver]\n    if: always()",
     "needs: [classify, lint, test, test-extra, coverage, msrv, doc, package, semver]\n    if: success()"),
    # 6. The plan script read from the checkout.
    ("the plan script read from the working tree", R,
     'git show "$BASE_SHA:.github/scripts/checks/rust-ci-inputs.sh" | bash -s -- "$BASE_SHA" "$HEAD_SHA" || true',
     'bash .github/scripts/checks/rust-ci-inputs.sh "$BASE_SHA" "$HEAD_SHA" || true'),
    ("the build judge read from the working tree", R,
     'judge=$(git show "$BASE_SHA:.github/scripts/checks/rust-build-inputs.sh")', "judge=$(cat .github/scripts/checks/rust-build-inputs.sh)"),
    ("the VM plan read from the working tree", "podman-lane.yml",
     'git show "$BASE_SHA:.github/scripts/checks/podman-lane-paths.sh" \\\n              | bash -s -- "$changed"',
     'cat .github/scripts/checks/podman-lane-paths.sh \\\n              | bash -s -- "$changed"'),
    # 7. An input that turns a check off, passed to a reusable.
    ("coverage turned off in the caller", "ci.yml", "coverage-threshold: 76", "coverage-threshold: 0"),
    ("doc warnings turned off in the caller", "ci.yml", "doc-warnings: true", "doc-warnings: false"),
    ("the MSRV check emptied in the caller", "ci.yml", 'msrv: "1.88"', 'msrv: ""'),
    ("an input nobody listed passed to the Rust pipeline", "ci.yml", "      doc-warnings: true", "      doc-warnings: true\n      run-tests: false"),
    ("an input passed to the line limit", "line-limit.yml",
     "    uses: ./.github/workflows/reusable-line-limit.yml\n    with:", "    uses: ./.github/workflows/reusable-line-limit.yml\n    with:\n      hard-limit: 5000"),
    ("an input passed to the DCO", "dco.yml", "    uses: ./.github/workflows/reusable-dco.yml", "    uses: ./.github/workflows/reusable-dco.yml\n    with:\n      skip: true"),
    # 8. A job added or removed; the list is closed and exempts nothing by name.
    ("a new job in the Rust pipeline", R, "  gate:\n", "  extra-check:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n\n  gate:\n"),
    ("a new job in the DCO caller", "dco.yml", "  dco:\n", "  extra:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n  dco:\n"),
    ("a job of the Rust pipeline removed", R, "  package:\n    name: Package check", "  package-gone:\n    name: Package check"),
    ("a new job named like an exempt gate", R, "  gate:\n", "  extra-platforms-gate-2:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n\n  gate:\n"),
    # 9. Structure and hygiene never depend on the plan.
    ("the folder limit put behind the plan", R,
     "      - name: folders hold at most twenty files\n", "      - name: folders hold at most twenty files\n        if: needs.classify.outputs.run != 'false'\n"),
    ("the vocabulary scan put behind the plan", R,
     "      - name: no vetoed vocabulary\n", "      - name: no vetoed vocabulary\n        if: needs.classify.outputs.run != 'false'\n"),
    ("a condition on the line limit", "reusable-line-limit.yml", "  line-limit:\n", "  line-limit:\n    if: github.event_name == 'push'\n"),
    ("a condition on the DCO", "reusable-dco.yml", "jobs:\n  dco:\n", "jobs:\n  dco:\n    if: github.event_name == 'push'\n"),
    ("a condition on the secret scan", "commit-hygiene.yml", "  secret-scan:\n", "  secret-scan:\n    if: github.event_name == 'push'\n"),
    ("a condition on the pin values check", "commit-hygiene.yml", "  pin-values:\n", "  pin-values:\n    if: github.event_name == 'push'\n"),
    ("the pin judge taken from the change instead of the base", "commit-hygiene.yml", 'install -m 0644 "base/$f"', 'install -m 0644 "head/$f"'),
    ("the pin judge run without isolating Python", "commit-hygiene.yml", "python3 -I judge/check-pins.py", "python3 judge/check-pins.py"),
    ("the pin judge run from the tree of the change", "commit-hygiene.yml", "python3 -I judge/check-pins.py", "python3 -I head/.github/scripts/hygiene/check-pins.py"),
    ("a condition on the commit identity check", "commit-hygiene.yml", "  commit-identity:\n", "  commit-identity:\n    if: github.event_name == 'push'\n"),
    # 10. The podman lane: the plan and the certification.
    ("the VM answer read as == 'true'", "podman-lane.yml",
     "if: needs.decide.outputs.engine != 'false'", "if: needs.decide.outputs.engine == 'true'"),
    ("Supported Podman majors reading another job", "podman-lane.yml",
     "NATIVE: ${{ needs.podman-native.result }}", "NATIVE: ${{ needs.decide.result }}"),
    ("the native lane also running on the schedule", "podman-lane.yml", "if: ${{ !cancelled() && (github.event_name == 'pull_request' || github.event_name == 'merge_group') && needs.decide.outputs.engine != 'false' }}", "if: ${{ !cancelled() && needs.decide.outputs.engine != 'false' }}"),
    ("the VM running on a pull request again", "podman-lane.yml", "if: ${{ !cancelled() && github.event_name != 'pull_request' && github.event_name != 'merge_group' }}\n    runs-on: ubuntu-24.04", "if: ${{ !cancelled() }}\n    runs-on: ubuntu-24.04"),
    ("the native lane given more than it needs", "podman-lane.yml", "    timeout-minutes: 60\n    permissions:\n      contents: read\n    env:\n", "    timeout-minutes: 60\n    permissions:\n      contents: read\n      id-token: write\n    env:\n"),
    ("the Quadlet tests of the native lane running after a failed install", "podman-lane.yml", "if: ${{ !cancelled() && steps.install.outcome == 'success' }}", "if: always()"),
    ("Supported Podman majors losing the plan", "podman-lane.yml", "needs: [decide, podman-vm, podman-native]", "needs: [decide, podman-vm]"),
]

SUF = "${{ github.event_name == 'pull_request' && github.base_ref == 'main' && ' (promotion)' || !(github.event_name == 'merge_group' || github.event_name == 'pull_request' && github.base_ref == 'develop') && format(' ({0})', github.event_name) || '' }}"

# Plants that required_names.py must catch: a required name that a push, a schedule,
# a dispatch or another workflow can also produce.
NAMES = [
    ("the line limit loses the run kind in its name", "reusable-line-limit.yml", "    name: line limit" + SUF, "    name: line limit"),
    ("the Rust gate loses the run kind in its name", R, "    name: Gate" + SUF, "    name: Gate"),
    ("Supported Podman majors loses the run kind in its name", "podman-lane.yml", "    name: Supported Podman majors" + SUF, "    name: Supported Podman majors"),
    ("the DCO loses the run kind in its name", "reusable-dco.yml", "    name: Signed-off-by present on every commit" + SUF, "    name: Signed-off-by present on every commit"),
    ("the workflow lint loses the run kind in its name", "reusable-workflow-lint.yml", "    name: workflow-lint" + SUF, "    name: workflow-lint"),
    ("a job whose id differs only by case produces the line limit again", "commit-hygiene.yml", "jobs:\n", "jobs:\n  Line-Limit:\n    uses: ./.github/workflows/reusable-line-limit.yml\n"),
    ("a required name that keeps a matrix value", "reusable-dco.yml", "    name: Signed-off-by present on every commit" + SUF, "    name: Signed-off-by present on every commit ${{ matrix.os }}"),
    ("the audit loses the run kind in its name", "reusable-rust-audit.yml", "    name: cargo audit" + SUF, "    name: cargo audit"),
    ("the secret scan loses the run kind in its name", "commit-hygiene.yml", "    name: secret scan" + SUF, "    name: secret scan"),
    ("the pin values lose the run kind in their name", "commit-hygiene.yml", "    name: pin values" + SUF, "    name: pin values"),
    ("the audit does not run in the merge queue", "audit.yml", "  merge_group:\n    types: [checks_requested]\n", ""),
    ("the commit hygiene does not run in the merge queue", "commit-hygiene.yml", "  merge_group:\n    types: [checks_requested]\n", ""),
    ("the audit cancelled by a newer run", "audit.yml", "  cancel-in-progress: false", "  cancel-in-progress: ${{ github.event_name == 'pull_request' }}"),
    ("the hygiene cancelled by a newer run", "commit-hygiene.yml", "  cancel-in-progress: false", "  cancel-in-progress: true"),
    ("a second workflow produces the line limit", "commit-hygiene.yml", "jobs:\n", "jobs:\n  line-limit:\n    uses: ./.github/workflows/reusable-line-limit.yml\n"),
    ("CI lists `edited`, which starts a second set of runs that cancels the first", "ci.yml", "    types: [opened, synchronize]\n", "    types: [opened, synchronize, edited]\n"),
    ("CI lists `reopened`", "ci.yml", "    types: [opened, synchronize]\n", "    types: [opened, synchronize, reopened]\n"),
    ("the DCO lists no types, and the default includes `reopened`", "dco.yml", "    types: [opened, synchronize]\n", ""),
    ("the audit lists `edited`", "audit.yml", "    types: [opened, synchronize]\n", "    types: [opened, synchronize, edited]\n"),
    ("the commit hygiene lists `reopened`", "commit-hygiene.yml", "    types: [opened, synchronize]\n", "    types: [opened, synchronize, reopened]\n"),
    ("the guard of the base stops listing `edited` and `reopened`, and the list of exceptions is stale", "main-guard.yml", "    types: [opened, synchronize, reopened, edited]\n", "    types: [opened, synchronize]\n"),
    ("the guard of the base gets a second job", "main-guard.yml", "jobs:\n", "jobs:\n  other:\n    runs-on: ubuntu-latest\n    steps:\n      - run: true\n"),
    ("the guard of the base gets an aggregator that always runs", "main-guard.yml", "jobs:\n", "jobs:\n  agg:\n    if: always()\n    runs-on: ubuntu-latest\n    steps:\n      - run: true\n"),
    ("the guard of the base cancels what is in progress", "main-guard.yml", "  cancel-in-progress: false", "  cancel-in-progress: ${{ github.event_name == 'pull_request' }}"),
    ("the Podman lane runs for any base, and `Develop` is read as develop", "podman-lane.yml", "    branches: [main, develop]\n  # Required check on develop", "  # Required check on develop"),
    ("the workflow lint runs for any base", "workflow-lint.yml", "    branches: [main, develop]\n  # Required check", "  # Required check"),
    ("a name with an expression nobody listed", "reusable-line-limit.yml", "    name: line limit" + SUF, "    name: line limit${{ github.ref_name }}"),
    ("a name that only a push could not tell from the pull request's", "reusable-line-limit.yml", "format(' ({0})', github.event_name)", "''"),
    ("a push run of the line limit gets the plain name", "reusable-line-limit.yml", "format(' ({0})', github.event_name)", "''"),
    ("the Podman lane promotion name is also given to the queue", "podman-lane.yml", "github.event_name == 'pull_request' && github.base_ref == 'main' && ' (promotion)'", "github.event_name != 'push' && ' (promotion)'"),
]


# What a Dependabot pull request does to a workflow, and what it must not be able to
# do. (description, file, old text, new text, replace every occurrence, must pass)
CK = "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1"
BUMP = "actions/checkout@" + "a" * 40
PINS = [
    ("a new commit for an action, with its new version comment", R, CK + " # v7.0.1", BUMP + " # v7.0.2", True, True),
    ("a new commit for another action, the cache", R, "actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0", "actions/cache@" + "b" * 40 + " # v6.2.0", True, True),
    ("a new commit for the checkout in the hygiene workflow", "commit-hygiene.yml", CK + " # v7.0.1", BUMP + " # v7.0.2", True, True),
    ("a new commit for the checkout in the cancel workflow", "reusable-cancel-others.yml", CK + " # v7.0.1", BUMP + " # v7.0.2", True, True),
    ("the action renamed, with the same commit", R, CK, "actions/cache@3d3c42e5aac5ba805825da76410c181273ba90b1", True, False),
    ("the pin removed", R, CK + " # v7.0.1", "actions/checkout", True, False),
    ("a tag in place of the commit", R, CK + " # v7.0.1", "actions/checkout@v7", True, False),
    ("a short hash in place of the commit", R, CK + " # v7.0.1", "actions/checkout@" + "3d3c42e5" + "a" * 31, True, False),
    ("an upper-case hash in place of the commit", R, CK + " # v7.0.1", "actions/checkout@" + "3D3C42E5AAC5BA805825DA76410C181273BA90B1", True, False),
    ("a branch in place of the commit", R, CK + " # v7.0.1", "actions/checkout@main", True, False),
    ("a different action under the same pin on one step only", R, CK + " # v7.0.1", "actions/setup-go@" + "3d3c42e5aac5ba805825da76410c181273ba90b1", False, False),
]


def main():
    if sys.argv[1] == "pins":
        if sys.argv[2] == "--count":
            print(len(PINS))
            return 0
        if sys.argv[2] == "--describe":
            print(PINS[int(sys.argv[3])][0])
            return 0
        if sys.argv[2] == "--passes":
            print("yes" if PINS[int(sys.argv[3])][5] else "no")
            return 0
        _d, name, old, new, every, _p = PINS[int(sys.argv[2])]
        path = os.path.join(sys.argv[3], ".github", "workflows", name)
        text = open(path).read()
        if old not in text:
            print(f"the pin plant {sys.argv[2]} does not find its text in {name}", file=sys.stderr)
            return 2
        open(path, "w").write(text.replace(old, new) if every else text.replace(old, new, 1))
        return 0
    if sys.argv[1] == "names":
        which = NAMES
        if sys.argv[2] == "--count":
            print(len(which))
            return 0
        if sys.argv[2] == "--describe":
            print(which[int(sys.argv[3])][0])
            return 0
        _d, name, old, new = which[int(sys.argv[2])]
        path = os.path.join(sys.argv[3], ".github", "workflows", name)
        text = open(path).read()
        if old not in text:
            print(f"the name plant {sys.argv[2]} does not find its text in {name}", file=sys.stderr)
            return 2
        open(path, "w").write(text.replace(old, new, 1))
        return 0
    if sys.argv[1] == "--count":
        print(len(PLANTS))
        return 0
    if sys.argv[1] == "--describe":
        print(PLANTS[int(sys.argv[2])][0])
        return 0
    index, tree = int(sys.argv[1]), sys.argv[2]
    _desc, name, old, new = PLANTS[index]
    path = os.path.join(tree, ".github", "workflows", name)
    text = open(path).read()
    if text.count(old) < 1:
        print(f"the plant {index} ({_desc}) does not find its text in {name}", file=sys.stderr)
        return 2
    open(path, "w").write(text.replace(old, new, 1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
