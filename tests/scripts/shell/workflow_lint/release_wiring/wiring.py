#!/usr/bin/env python3
"""The wiring of release.yml that no single script exercises.

Usage: wiring.py [WORKFLOW]   (prints one line per problem, exit 1 if any)

What a release needs of its jobs is in the `needs`, the permissions, the paths
and the names, and a wrong value in one of them only shows once everything has
been built. These checks read the workflow and compare it with the files the
release reads at the end (`verify-provenance.sh`), so a change on one side that
is not made on the other fails here.
"""
import fnmatch
import os
import re
import sys

import yaml

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "../../../../.."))
ART = "${{ runner.temp }}/artifacts"

# What each job is allowed to be granted. Anything else is a change to decide.
PERMISSIONS = {
    "verify": {"contents": "read"},
    "build": {"contents": "read"},
    "build-deb": {"contents": "read"},
    "sign": {"contents": "read"},
    "sbom": {"contents": "read"},
    "attest": {"contents": "read", "id-token": "write", "attestations": "write"},
    "release": {"contents": "write", "attestations": "read"},
}
# Release-job steps that name their files by absolute path; every other step of
# the job names them relative to the artifact directory and must run in it.
ABSOLUTE = ("Gather the artifacts", "Verify the provenance")


def uses(step, action):
    return action in str(step.get("uses", ""))


def provenance_files():
    """The names `verify-provenance.sh` asks an attestation for, as examples."""
    text = open(f"{REPO}/.github/scripts/release/verify-provenance.sh").read()
    loop = re.search(r"for file in (.*?); do", text, re.S).group(1)
    return [w.replace("*", "X") for w in loop.replace("\\", " ").split()]


def problems(path):
    spec = yaml.safe_load(open(path))
    jobs = spec["jobs"]
    out = []

    def bad(message):
        out.append(message)

    # 1. The release waits for every job that makes something it publishes.
    producers = {
        name
        for name, job in jobs.items()
        if name not in ("release", "verify")
    }
    needs = set(jobs["release"].get("needs") or [])
    for name in sorted(producers - needs):
        bad(f"release does not wait for `{name}`")
    if not any("verify" in (jobs[n].get("needs") or []) for n in needs if n in jobs):
        bad("nothing the release waits for follows `verify`")

    # 2. Permissions, exactly, and the two that mint tokens only on `attest`.
    for name, job in jobs.items():
        granted = job.get("permissions")
        if granted != PERMISSIONS.get(name):
            bad(f"`{name}` has permissions {granted}, expected {PERMISSIONS.get(name)}")
        if name != "attest" and any((granted or {}).get(k) == "write" for k in ("id-token", "attestations")):
            bad(f"`{name}` can mint a token")

    # 3. The release job: names relative to the artifact directory, a tag that
    # exists, and the directory checked before anything is signed.
    steps = jobs["release"]["steps"]
    runs = [s for s in steps if "run" in s]
    if not any("check-artifact-dir.sh" in s["run"] and re.search(r'artifacts" release', s["run"]) for s in runs):
        bad("the release job does not check the artifact directory before signing")
    for step in runs:
        name = str(step.get("name", ""))
        if name.startswith(ABSOLUTE):
            continue
        if step.get("working-directory") != ART:
            bad(f"release step `{name}` does not run in the artifact directory")
    create = next((s for s in runs if "gh release create" in s["run"]), None)
    if create is None or "--verify-tag" not in create["run"]:
        bad("`gh release create` does not pass --verify-tag")

    # 4. What `attest` covers is what `verify-provenance.sh` asks for, and what
    # it downloads is every artifact the build jobs upload.
    attest = jobs["attest"]["steps"]
    subjects = next(s for s in attest if uses(s, "attest-build-provenance"))["with"]["subject-path"].splitlines()
    globs = [g.strip().removeprefix(ART + "/") for g in subjects if g.strip()]
    for name in provenance_files():
        if not any(fnmatch.fnmatchcase(name, g) for g in globs):
            bad(f"`verify-provenance.sh` asks for {name}, which `attest` does not cover")
    download = next(s for s in attest if uses(s, "download-artifact"))
    pattern = download["with"].get("pattern", "")
    if download["with"].get("path") != ART:
        bad("`attest` does not download into the artifact directory below runner.temp")
    check = next((i for i, s in enumerate(attest) if "check-artifact-dir.sh" in str(s.get("run", "")) and " attest" in s["run"]), None)
    attesting = next(i for i, s in enumerate(attest) if uses(s, "attest-build-provenance"))
    if check is None or check > attesting:
        bad("`attest` does not check the artifact names before it attests")
    sign_matrix = jobs["sign"]["strategy"]["matrix"]["asset"]
    build_assets = [i["asset"] for i in jobs["build"]["strategy"]["matrix"]["include"]]
    deb_name = next(s for s in jobs["build-deb"]["steps"] if uses(s, "upload-artifact"))["with"]["name"]
    debs = [deb_name.replace("${{ matrix.arch }}", i["arch"]) for i in jobs["build-deb"]["strategy"]["matrix"]["include"]]
    for name in build_assets + debs:
        if not fnmatch.fnmatchcase(name, pattern):
            bad(f"`attest` downloads `{pattern}`, which does not match the artifact {name}")

    # 5. The signing matrix is the set of binaries, and what it uploads cannot
    # collide with anything else or be written outside the uploaded folder.
    if sorted(sign_matrix) != sorted(build_assets):
        bad("the signing matrix is not the set of binaries the build makes")
    sign = jobs["sign"]["steps"]
    down = next(s for s in sign if uses(s, "download-artifact"))["with"]
    up = next(s for s in sign if uses(s, "upload-artifact"))["with"]
    if up["path"] != f"{down['path']}/${{{{ matrix.asset }}}}.sig":
        bad("the signature is not uploaded from the folder the binary was downloaded to")
    names = [str(i) for i in build_assets] + debs + ["sbom"] + [f"sig-{a}" for a in sign_matrix]
    if up["name"] != "sig-${{ matrix.asset }}":
        bad("the signature artifact is not named `sig-<asset>`")
    if len(set(names)) != len(names):
        bad("two artifacts would carry the same name")
    return out


if __name__ == "__main__":
    found = problems(sys.argv[1] if len(sys.argv) > 1 else f"{REPO}/.github/workflows/release.yml")
    print("\n".join(found))
    sys.exit(1 if found else 0)
