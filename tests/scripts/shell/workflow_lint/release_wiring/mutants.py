#!/usr/bin/env python3
"""Mutate release.yml one wiring fact at a time; each must be reported.

Usage: mutants.py   (prints `killed N`, `SURVIVED name` per survivor)
"""
import copy
import os
import sys
import tempfile

import yaml

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import wiring  # noqa: E402

SRC = f"{wiring.REPO}/.github/workflows/release.yml"
spec = yaml.safe_load(open(SRC))
ART = wiring.ART


def step(job, needle):
    return next(s for s in spec_m["jobs"][job]["steps"] if needle in str(s.get("name", "")) or needle in str(s.get("uses", "")))


MUTANTS = {}


def mutant(fn):
    MUTANTS[fn.__name__] = fn
    return fn


def drop_need(name):
    def fn(m):
        m["jobs"]["release"]["needs"].remove(name)
    return fn


for _n in ("build", "sign", "build-deb", "sbom", "attest"):
    MUTANTS[f"release-without-needs-{_n}"] = drop_need(_n)
for _j in ("verify", "build", "build-deb", "sign", "sbom", "release"):
    MUTANTS[f"id-token-on-{_j}"] = (lambda j: lambda m: m["jobs"][j]["permissions"].update({"id-token": "write"}))(_j)
for _j in ("build", "sign", "release"):
    MUTANTS[f"attestations-write-on-{_j}"] = (lambda j: lambda m: m["jobs"][j]["permissions"].update({"attestations": "write"}))(_j)
MUTANTS["attest-without-id-token"] = lambda m: m["jobs"]["attest"]["permissions"].pop("id-token")
MUTANTS["release-contents-read"] = lambda m: m["jobs"]["release"]["permissions"].update({"contents": "read"})


@mutant
def no_verify_tag(m):
    s = step("release", "Create GitHub Release")
    s["run"] = "\n".join(l for l in s["run"].splitlines() if "--verify-tag" not in l)


@mutant
def no_release_dir_check(m):
    s = step("release", "Gather the artifacts")
    s["run"] = "\n".join(l for l in s["run"].splitlines() if not l.endswith("release"))


for _name in (
    "Verify the Linux binaries", "Generate checksums", "Sign SHA256SUMS",
    "Verify every signature", "Verify every file the installers fetch", "Create GitHub Release",
):
    MUTANTS[f"no-working-directory-{_name.split()[1]}"] = (
        lambda n: lambda m: step("release", n).pop("working-directory")
    )(_name)


def narrow_subject(index):
    def fn(m):
        s = step("attest", "attest-build-provenance")
        lines = s["with"]["subject-path"].splitlines()
        del lines[index]
        s["with"]["subject-path"] = "\n".join(lines) + "\n"
    return fn


for _i in range(4):
    MUTANTS[f"attest-subject-{_i}-missing"] = narrow_subject(_i)
MUTANTS["attest-subject-only-linux-x86"] = lambda m: step("attest", "attest-build-provenance")["with"].update(
    {"subject-path": f"{ART}/podup-linux-x86_64\n{ART}/podup-linux-arm64\n"})
MUTANTS["attest-pattern-narrower"] = lambda m: step("attest", "download-artifact")["with"].update({"pattern": "podup-linux-*"})
MUTANTS["attest-downloads-into-the-workspace"] = lambda m: step("attest", "download-artifact")["with"].update({"path": "${{ github.workspace }}"})
MUTANTS["attest-without-the-name-check"] = lambda m: m["jobs"]["attest"]["steps"].remove(step("attest", "Check the artifacts"))
MUTANTS["attest-checks-the-names-after-attesting"] = lambda m: m["jobs"]["attest"]["steps"].append(m["jobs"]["attest"]["steps"].pop(2))
MUTANTS["sign-matrix-short"] = lambda m: m["jobs"]["sign"]["strategy"]["matrix"]["asset"].pop()
MUTANTS["sig-written-elsewhere"] = lambda m: step("sign", "upload-artifact")["with"].update({"path": "${{ matrix.asset }}.sig"})
MUTANTS["sig-artifact-name-collides"] = lambda m: step("sign", "upload-artifact")["with"].update({"name": "${{ matrix.asset }}"})
MUTANTS["deb-artifact-name-collides"] = lambda m: step("build-deb", "upload-artifact")["with"].update({"name": "podup-deb"})

dead, alive = 0, []
for name, fn in MUTANTS.items():
    spec_m = copy.deepcopy(spec)
    fn(spec_m)
    with tempfile.NamedTemporaryFile("w", suffix=".yml", delete=False) as tmp:
        yaml.safe_dump(spec_m, tmp)
    try:
        if wiring.problems(tmp.name):
            dead += 1
        else:
            alive.append(name)
    finally:
        os.unlink(tmp.name)
print(f"killed {dead}")
for name in alive:
    print(f"SURVIVED {name}")
sys.exit(1 if alive else 0)
