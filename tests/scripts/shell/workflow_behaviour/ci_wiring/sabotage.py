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
IF_TEST = "    if: ${{ !cancelled() && (needs.classify.result != 'success' || needs.classify.outputs.run != 'false') }}\n    runs-on: ubuntu-latest\n    # Bound: worst of the last three successful runs on 2026-09-02 was 106s"

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
        "if: ${{ !cancelled() && (needs.classify.result != 'success' || needs.classify.outputs.run != 'false') }}", "if: false")),
    ("a fixed false on the Rust pipeline in its caller", "ci.yml", "  rust:\n", "  rust:\n    if: false\n"),
    ("a duplicate `if` on the Linux tests, which PyYAML's safe_load reads as the last one", R,
     "  test:\n    name: Test\n", "  test:\n    name: Test\n    if: true\n"),
    ("a duplicate key in the gate", R, "  gate:\n", "  gate:\n    runs-on: ubuntu-latest\n"),
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
    ("a condition on the commit identity check", "commit-hygiene.yml", "  commit-identity:\n", "  commit-identity:\n    if: github.event_name == 'push'\n"),
    # 10. The podman lane: the plan and the certification.
    ("the VM answer read as == 'true'", "podman-lane.yml",
     "if: needs.decide.outputs.engine != 'false'", "if: needs.decide.outputs.engine == 'true'"),
    ("Supported Podman majors reading another job", "podman-lane.yml",
     "RESULT: ${{ needs.podman-vm.result }}", "RESULT: ${{ needs.decide.result }}"),
    ("Supported Podman majors losing the plan", "podman-lane.yml", "needs: [decide, podman-vm]", "needs: podman-vm"),
]

SUF = "${{ github.event_name == 'pull_request' && github.base_ref == 'main' && ' (promotion)' || github.event_name != 'pull_request' && github.event_name != 'merge_group' && format(' ({0})', github.event_name) || '' }}"

# Plants that required_names.py must catch: a required name that a push, a schedule,
# a dispatch or another workflow can also produce.
NAMES = [
    ("the line limit loses the run kind in its name", "reusable-line-limit.yml", "    name: line limit" + SUF, "    name: line limit"),
    ("the Rust gate loses the run kind in its name", R, "    name: Gate" + SUF, "    name: Gate"),
    ("Supported Podman majors loses the run kind in its name", "podman-lane.yml", "    name: Supported Podman majors" + SUF, "    name: Supported Podman majors"),
    ("the DCO loses the run kind in its name", "reusable-dco.yml", "    name: Signed-off-by present on every commit" + SUF, "    name: Signed-off-by present on every commit"),
    ("the workflow lint loses the run kind in its name", "reusable-workflow-lint.yml", "    name: workflow-lint" + SUF, "    name: workflow-lint"),
    ("a second workflow produces the line limit", "commit-hygiene.yml", "jobs:\n", "jobs:\n  line-limit:\n    uses: ./.github/workflows/reusable-line-limit.yml\n"),
    ("a push run of the line limit gets the plain name", "reusable-line-limit.yml", "format(' ({0})', github.event_name)", "''"),
    ("the Podman lane promotion name is also given to the queue", "podman-lane.yml", "github.event_name == 'pull_request' && github.base_ref == 'main' && ' (promotion)'", "github.event_name != 'push' && ' (promotion)'"),
]


def main():
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
