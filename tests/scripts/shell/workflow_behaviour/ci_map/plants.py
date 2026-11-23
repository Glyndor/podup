#!/usr/bin/env python3
"""The sabotages the map must catch, as data: (description, directory, file, old text, new text).

`directory` is `checks` (a copy of .github/scripts/checks) or `workflows` (a copy of
.github/workflows). `plants.py --count`, `--describe N`, or `N CHECKS WORKFLOWS` to
apply the N-th to the two copies; it fails when its text is not there.
"""
import os
import sys

PLANTS = [
    ("Cargo.lock moved to 'nothing to run' for the Rust lanes", "checks", "rust-ci-inputs.sh",
     "\t\tCargo.* | */Cargo.*) answer", "\t\tCargo.lock) other=true ;;\n\t\tCargo.* | */Cargo.*) answer"),
    ("deny.toml (the audit configuration) moved to 'nothing to run'", "checks", "rust-ci-inputs.sh",
     'deny.toml) answer true false "$path is read by the supply-chain test" ;;', "deny.toml) other=true ;;"),
    ("the installer moved to 'nothing to run'", "checks", "rust-ci-inputs.sh",
     'install.sh | install.ps1) answer true false "$path is read by the release-key test" ;;', "install.sh | install.ps1) other=true ;;"),
    ("the cargo configuration moved to 'nothing to run'", "checks", "rust-ci-inputs.sh",
     'rustfmt.toml | .cargo/*) answer true false "$path changes what fmt and clippy do" ;;',
     'rustfmt.toml) answer true false "$path changes what fmt and clippy do" ;;\n\t\t.cargo/*) other=true ;;'),
    ("the toolchain pin moved to 'nothing to run' for the Rust lanes", "checks", "rust-ci-inputs.sh",
     '*) answer true false "$path is not a known input of the Rust lanes" ;;', "*) other=true ;;"),
    ("the build script moved to 'not a build input'", "checks", "rust-build-inputs.sh",
     "tests/* | docs/* | bench/* | fuzz/* | debian/* | \\\n\t\tREADME.md", "tests/* | docs/* | bench/* | fuzz/* | debian/* | build.rs | \\\n\t\tREADME.md"),
    ("the lockfile moved to 'not a build input'", "checks", "rust-build-inputs.sh",
     "tests/* | docs/* | bench/* | fuzz/* | debian/* | \\\n\t\tREADME.md", "tests/* | docs/* | bench/* | fuzz/* | debian/* | Cargo.lock | \\\n\t\tREADME.md"),
    ("the crate moved to 'the VM cannot see it'", "checks", "podman-lane-paths.sh",
     "\t\tfuzz/* | bench/* | tests/scripts/*) ;;", "\t\tfuzz/* | bench/* | tests/scripts/* | internal/*) ;;"),
    ("the dependency graph moved to 'no Debian package'", "checks", "debian-ci-inputs.sh",
     '\t\tCargo.* | */Cargo.*) answer true "$path changes the dependency graph" ;;', "\t\tCargo.* | */Cargo.*) ;;"),
    ("the packaging moved to 'no Debian package'", "checks", "debian-ci-inputs.sh",
     '\t\tdebian/*) answer true "$path changes the packaging" ;;', "\t\tdebian/*) ;;"),
    ("a workflow moved to 'the lint cannot see it'", "checks", "workflow-lint-inputs.sh",
     '\t\t.github/workflows/*) answer true "$path changes a workflow the lint reads" ;;', "\t\t.github/workflows/*) ;;"),
    ("a check script moved to 'the lint cannot see it'", "checks", "workflow-lint-inputs.sh",
     '\t\t.github/scripts/*) answer true "$path changes a check the lint runs" ;;', "\t\t.github/scripts/*) ;;"),
    ("the reusable Rust pipeline removed from the suite that watches it", "workflows", "lint-shell.yml",
     '      - ".github/workflows/reusable-rust-ci.yml"\n', ""),
    ("the caller-with-defaults check removed from the suite that watches it", "workflows", "lint-shell.yml",
     '      - ".github/scripts/checks/check-caller-with-defaults.py"\n', ""),
    ("a path added that starts the binary budget for nothing", "workflows", "binary-budget.yml",
     '      - "internal/**"\n', '      - "internal/**"\n      - "docs/**"\n'),
    ("the Debian lane no longer watches the lockfile", "workflows", "debian-build.yml", '      - "Cargo.lock"\n', ""),
]


# The files each plant can move, so the case judges those and not the whole tree.
AFFECTS = {
    0: "Cargo.lock", 1: "deny.toml", 2: "install.sh", 3: ".cargo/audit.toml", 4: ".gitignore",
    5: "build.rs", 6: "Cargo.lock", 7: "internal/lib.rs", 8: "Cargo.lock", 9: "debian/rules",
    10: ".github/workflows/ci.yml", 11: ".github/scripts/checks/folder-limit.sh",
    12: "Cargo.toml", 13: "Cargo.toml", 14: "Cargo.toml", 15: "Cargo.toml",
}


def main():
    if sys.argv[1] == "--count":
        print(len(PLANTS))
        return 0
    if sys.argv[1] == "--describe":
        print(PLANTS[int(sys.argv[2])][0])
        return 0
    if sys.argv[1] == "--affects":
        print(AFFECTS.get(int(sys.argv[2]), "Cargo.toml"))
        return 0
    desc, kind, name, old, new = PLANTS[int(sys.argv[1])]
    base = sys.argv[2] if kind == "checks" else sys.argv[3]
    path = os.path.join(base, name)
    text = open(path).read()
    if old not in text:
        print(f"the plant {sys.argv[1]} ({desc}) does not find its text in {name}", file=sys.stderr)
        return 2
    open(path, "w").write(text.replace(old, new))  # every occurrence: a path list is written twice, for push and for pull_request
    return 0


if __name__ == "__main__":
    sys.exit(main())
