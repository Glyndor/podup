#!/usr/bin/env python3
"""The wiring of release.yml and reusable-release-build.yml that no single script exercises.

Usage: wiring.py [WORKFLOW]   (prints one line per problem, exit 1 if any)

What a release needs of its jobs is in the `needs`, the permissions, the paths, the names, the commit
they check out and the way Python is started, and a wrong value in one of them only shows once
everything has been built. These checks read the workflows and compare them with the files the release
reads (`expected-assets.py`, `verify-provenance.sh`), so a change on one side that is not made on the
other fails here. The reusable that holds the build jobs is read from the folder of WORKFLOW.
"""
import fnmatch
import os
import re
import subprocess
import sys

import yaml

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "../../../../.."))
ART = "${{ runner.temp }}/artifacts"
REL = "${{ runner.temp }}/release"
COMMIT = "${{ needs.verify.outputs.commit }}"
# What every checkout after `verify` names: the tag, never a revision that an input or an output carries
# (CodeQL reads that as code of a revision chosen by input, run in a workflow that can write the default
# branch's cache). That it is the commit `verify` named is asserted right after, in every job.
TAG_REF = "refs/tags/${{ github.event_name == 'workflow_dispatch' && inputs.tag || github.ref_name }}"
TAG_REF_INPUT = "refs/tags/${{ inputs.tag }}"
# The caches an Actions workflow can read and write: nothing a release restores or saves is shared with
# the runs of pull requests and of the other workflows, so what is built is what the tag holds.
CACHE_INPUTS = {"cache", "cache-dependency-path", "cache-from", "cache-to", "restore-keys", "enableCrossOsArchive"}

# What each job is allowed to be granted. Anything else is a change to decide.
PERMISSIONS = {
    "verify": {"contents": "read"},
    "builds": {"contents": "read"},
    "sign": {"contents": "read", "attestations": "read"},
    "sign-packages": {"contents": "read", "attestations": "read"},
    "attest": {"contents": "read", "id-token": "write", "attestations": "write"},
    "seal": {"contents": "read", "attestations": "read"},
    "release": {"contents": "write", "attestations": "read"},
}
NOT_DRY = "${{ !inputs.dry_run }}"
TAG_CHECK = "refs/tags/${TAG}^{commit}"
# The jobs that hold the key, which have a `release` environment and never the right to write.
KEY_JOBS = ("sign", "sign-packages", "seal")
VERSION = "1.2.3"


def uses(step, action):
    return action in str(step.get("uses", ""))


def expected(which):
    out = subprocess.run(
        ["python3", "-I", f"{REPO}/.github/scripts/release/expected-assets.py", VERSION, which],
        capture_output=True, text=True, check=True,
    ).stdout
    return out.split()


def needs_of(job):
    value = job.get("needs") or []
    return [value] if isinstance(value, str) else list(value)


def closure(jobs, name):
    seen, todo = set(), [name]
    while todo:
        for n in needs_of(jobs[todo.pop()]):
            if n not in seen:
                seen.add(n)
                todo.append(n)
    return seen


def attest_condition(job):
    """The `if` of a job that waits for `attest`: a skipped attestation is accepted in a rehearsal and only there."""
    others = [n for n in needs_of(job) if n != "attest"]
    ok = " && ".join(f"needs.{n}.result == 'success'" if n.isidentifier() else f"needs['{n}'].result == 'success'" for n in others)
    return "${{ !cancelled() && " + ok + " && (needs.attest.result == 'success' || inputs.dry_run && needs.attest.result == 'skipped') }}"


OVERLAY = "Take the release scripts from the commit that started the rehearsal"


def first_script(steps, skip=None):
    """The first step that runs a script of the checkout. The step that puts the scripts of the commit that
    started a rehearsal in place runs none of the tag's: it only reads them out of git."""
    for i, s in enumerate(steps):
        text = str(s.get("run", ""))
        if i != skip and s.get("name") != OVERLAY and (".github/scripts" in text or "bash .github" in text):
            return i
    return None


def runs(job):
    return [s for s in job.get("steps") or [] if "run" in s]


def python_without_isolation(steps, where):
    """A Python started without -I: the script's own directory goes first on its path, and that
    directory is part of the tree of the tag."""
    out = []
    for s in steps:
        for line in str(s.get("run", "")).splitlines():
            code = line.split("#", 1)[0]
            for m in re.finditer(r'(?:^|[\s;|&(])(?:python3?|"?\$venv_py"?|\$py)\s+(-\S+\s+)*', code):
                rest = code[m.end():]
                flags = code[m.start():m.end()]
                if "-I" in flags.split():
                    continue
                # `command -v python` and a variable named py are not a start of Python.
                head = code[:m.start()]
                if re.search(r"command\s+-v\s*$", head) or re.match(r"\s*py=", code):
                    continue
                if not rest.strip() and not flags.strip().endswith(("python", "python3")):
                    continue
                out.append(f"{where}: `{s.get('name', '')}` starts Python without -I: {code.strip()[:80]}")
    return out


def problems(path):
    spec = yaml.safe_load(open(path))
    jobs = spec["jobs"]
    reusable_path = os.path.join(os.path.dirname(path), "reusable-release-build.yml")
    reusable = yaml.safe_load(open(reusable_path))
    rjobs = reusable["jobs"]
    verify_spec = yaml.safe_load(open(os.path.join(os.path.dirname(path), "reusable-release-verify.yml")))
    vjobs = verify_spec["jobs"]
    out = []

    def bad(message):
        out.append(message)

    # 1. Permissions, exactly, and the two that mint tokens only on `attest`; the reusable reads only.
    for name, job in jobs.items():
        granted = job.get("permissions")
        if granted != PERMISSIONS.get(name):
            bad(f"`{name}` has permissions {granted}, expected {PERMISSIONS.get(name)}")
        if name != "attest" and any((granted or {}).get(k) == "write" for k in ("id-token", "attestations")):
            bad(f"`{name}` can mint a token")
    for name in PERMISSIONS:
        if name not in jobs:
            bad(f"release.yml has no job `{name}`")
    if reusable.get("permissions") != {"contents": "read"}:
        bad("the reusable of the builds does not hold `contents: read` and nothing else at the top")
    for name, job in rjobs.items():
        if "permissions" in job and job["permissions"] != {"contents": "read"}:
            bad(f"reusable job `{name}` is granted {job['permissions']}")
        if "environment" in job or "secrets" in str(job):
            bad(f"reusable job `{name}` has an environment or reaches for secrets")
    # The key jobs have the environment; the publishing job has the right to write and no environment of the key.
    for name in KEY_JOBS:
        if jobs[name].get("environment") != "release":
            bad(f"`{name}` holds the key without `environment: release`")
        if "write" in (jobs[name].get("permissions") or {}).values():
            bad(f"`{name}` holds the key and a permission to write")
    if "environment" in jobs["release"] or "secrets" in str(jobs["release"]).replace("secrets.GITHUB_TOKEN", ""):
        bad("`release` writes the release and must not reach for the key or an environment")

    # 2. The graph: everything that makes something waits for `verify`, the sealing job waits for every
    # job that makes something it gathers, and the publishing job waits for the sealing one.
    for name in jobs:
        if name != "verify" and "verify" not in needs_of(jobs[name]) and name != "release":
            bad(f"`{name}` does not wait for `verify`")
    producers = {n for n in jobs if n not in ("seal", "release", "verify")}
    reach = closure(jobs, "seal") | set(needs_of(jobs["seal"]))
    for name in sorted(producers - reach):
        bad(f"seal does not wait for `{name}`")
    if "seal" not in needs_of(jobs["release"]):
        bad("release does not wait for `seal`")
    call = jobs["builds"]
    if call.get("uses") != "./.github/workflows/reusable-release-build.yml":
        bad("`builds` does not call the reusable of the builds")
    if (call.get("with") or {}) != {"commit": COMMIT, "tag": "${{ needs.verify.outputs.tag }}", "version": "${{ needs.verify.outputs.version }}",
                                 "dry_run": "${{ github.event_name == 'workflow_dispatch' && inputs.dry_run }}"}:
        bad("`builds` is not called with the commit, the tag and the version `verify` named, and the dry_run of the run")

    # 3. The commit: every checkout after `verify` is the tag of the run, never a bare revision, and every job
    # asserts right after that what it checked out is the commit `verify` named.
    for label, group, want in (("release.yml", jobs, TAG_REF), ("the builds", rjobs, TAG_REF_INPUT), ("the verification", {"audit": vjobs["audit"]}, TAG_REF_INPUT)):
        for name, job in group.items():
            steps = job.get("steps") or []
            for i, s in enumerate(steps):
                if not uses(s, "actions/checkout"):
                    continue
                if name == "verify":
                    continue
                if (s.get("with") or {}).get("ref") != want:
                    bad(f"{label}: `{name}` checks out {(s.get('with') or {}).get('ref')!r}, not the tag of the run")
                rest = [x for x in steps[i + 1:] if x.get("name") not in (OVERLAY, "Trust the workspace")]
                follow = str(rest[0].get("run", "")) if rest else ""
                if 'git rev-parse HEAD' not in follow:
                    bad(f"{label}: `{name}` does not check that its checkout is the verified commit")
    out_names = set(((verify_spec.get("on", verify_spec.get(True)) or {}).get("workflow_call") or {}).get("outputs") or {})
    if out_names != {"commit", "version", "tag"}:
        bad(f"the verification outputs {sorted(out_names)}, expected commit, version and tag")
    if set(vjobs["verify"].get("outputs") or {}) != {"commit", "version", "tag"} or vjobs["audit"].get("outputs"):
        bad("the commit, the version and the tag come from the verify job, and the audit has no outputs")
    if jobs["verify"].get("uses") != "./.github/workflows/reusable-release-verify.yml" or \
            (jobs["verify"].get("with") or {}).get("tag") != "${{ github.event_name == 'workflow_dispatch' && inputs.tag || github.ref_name }}":
        bad("`verify` is not the verification, called with the tag of the run")
    if "audit" not in needs_of(vjobs["audit"]) and "verify" not in needs_of(vjobs["audit"]):
        bad("the audit does not wait for the verification")
    if any(re.search(r"\bcargo (install|build|audit|run|test)\b", str(st.get("run", ""))) for st in vjobs["verify"]["steps"]):
        bad("the job that names the commit runs something of other people's")
    pin = next((i for i, st in enumerate(vjobs["verify"]["steps"]) if st.get("id") == "pin"), None)
    if pin is None:
        bad("the verify job does not name the commit in a step called pin")
    refuse = next((st for st in vjobs["verify"]["steps"] if "Refuse an already published" in str(st.get("name", ""))), None)
    if refuse is None or "if" in refuse or (refuse.get("env") or {}).get("MODE") != "${{ inputs.dry_run && 'rehearsal' || 'release' }}" \
            or str(refuse.get("run", "")).strip() != 'bash .github/scripts/release/check-unpublished.sh "$TAG" "$MODE"':
        bad("a published tag stops a real release and not a rehearsal: by the script, with the mode taken from dry_run, and no condition")
    if "refs/tags/${{ inputs.tag }}" != next(st for st in vjobs["verify"]["steps"] if uses(st, "actions/checkout"))["with"]["ref"]:
        bad("the verification does not check out the tag it was given")

    # 3b. Every job that holds the key, mints a token or writes the release looks the tag up again before it
    # runs a script of the checkout, and compares what it finds with the commit that was verified.
    for name in KEY_JOBS + ("attest", "release"):
        st = jobs[name]["steps"]
        check = next((i for i, x in enumerate(st) if TAG_CHECK in str(x.get("run", "")) and "git fetch" in str(x.get("run", ""))), None)
        if check is None:
            bad(f"`{name}` does not look the tag up again before it uses the checkout")
            continue
        script = first_script(st, skip=check)
        if script is not None and script < check:
            bad(f"`{name}` runs a script of the checkout before it looks the tag up again")
        if (st[check].get("env") or {}).get("COMMIT") != COMMIT or (st[check].get("env") or {}).get("TAG") != "${{ needs.verify.outputs.tag }}":
            bad(f"`{name}` compares the tag with something other than the commit `verify` named")
    # 3c. The jobs that sign wait for the attestation and verify, before they sign, what they downloaded;
    # the attestation and what depends on it are skipped in a rehearsal and in no other run.
    if jobs["attest"].get("if") != NOT_DRY:
        bad("`attest` is not skipped in a rehearsal and only there")
    for name in ("sign", "sign-packages", "seal"):
        job = jobs[name]
        if "attest" not in needs_of(job):
            bad(f"`{name}` does not wait for `attest`")
        elif job.get("if") != attest_condition(job):
            bad(f"`{name}` does not carry the condition that accepts a skipped attestation in a rehearsal only")
        st = job["steps"]
        prov = next((i for i, x in enumerate(st) if "verify-provenance.sh" in str(x.get("run", ""))), None)
        signing = next((i for i, x in enumerate(st) if "sign.py" in str(x.get("run", ""))), None)
        if prov is None or signing is None or prov > signing:
            bad(f"`{name}` signs before it verifies the provenance of what it signs")
        elif st[prov].get("if") != NOT_DRY or st[prov].get("continue-on-error"):
            bad(f"the provenance step of `{name}` can be skipped in a real run")
        else:
            env = st[prov].get("env") or {}
            if env.get("SOURCE_REF") != "refs/tags/${{ needs.verify.outputs.tag }}" or env.get("SOURCE_COMMIT") != COMMIT:
                bad(f"`{name}` does not verify the provenance for the tag and the commit `verify` named")
    for name in ("sign", "sign-packages"):
        prov = next((x for x in jobs[name]["steps"] if "verify-provenance.sh" in str(x.get("run", ""))), {})
        if not str((prov.get("env") or {}).get("NAME", "")).startswith("${{") or '"$NAME"' not in str(prov.get("run", "")):
            bad(f"`{name}` does not verify the one file it signs")

    # 4. The sealing job: names checked before provenance, provenance before signing, nothing skippable.
    seal = jobs["seal"]["steps"]
    sruns = runs(jobs["seal"])
    def at(job_steps, needle):
        for i, s in enumerate(job_steps):
            if needle in str(s.get("run", "")):
                return i
        return None
    staged, prov, sign = at(seal, "exact \"$VERSION\" staged"), at(seal, "verify-provenance.sh"), at(seal, "sign.py")
    final = at(seal, "exact \"$VERSION\" published")
    if None in (staged, prov, sign, final) or not staged < prov < sign < final:
        bad("seal does not check the names, then the provenance, then sign, then check the whole set")
    for needle in ("exact \"$VERSION\" staged", "exact \"$VERSION\" published"):
        i = at(seal, needle)
        if i is not None and ("if" in seal[i] or seal[i].get("continue-on-error")):
            bad(f"the seal step `{seal[i].get('name')}` can be skipped")
    if (jobs["seal"].get("outputs") or {}).get("sums") != "${{ steps.sums.outputs.sha256 }}" or at(seal, "sha256sum SHA256SUMS") is None:
        bad("seal does not report the digest of the checksums it signed")
    proven = seal[prov] if prov is not None else {}
    if (proven.get("env") or {}).get("SOURCE_REF") != "refs/tags/${{ needs.verify.outputs.tag }}" or (proven.get("env") or {}).get("SOURCE_COMMIT") != COMMIT:
        bad("the provenance is not verified for the tag and the commit `verify` named")
    for step in sruns:
        if str(step.get("name", "")).startswith(("Verify the Linux", "Generate checksums", "Sign SHA256SUMS", "Verify every signature", "Verify every file the installers")):
            if step.get("working-directory") != ART:
                bad(f"seal step `{step.get('name')}` does not run in the artifact directory")
    if at(seal, "xargs sha256sum") is None or at(seal, "checksummed") is None:
        bad("the checksums are not made from the list the version gives")
    upload = next((s for s in seal if uses(s, "upload-artifact")), None)
    if upload is None or upload["with"].get("name") != "release-set" or upload["with"].get("path") != f"{ART}/*":
        bad("seal does not upload the sealed set as `release-set`")

    # 5. The publishing job: the exact set again, the signatures again, the tag again, explicit names,
    # `--verify-tag`, and nothing published in a rehearsal.
    rel = jobs["release"]["steps"]
    down = next((s for s in rel if uses(s, "download-artifact")), None)
    if down is None or "merge-multiple" in down.get("with", {}) or down["with"].get("name") != "release-set" or down["with"].get("path") != REL:
        bad("release does not download `release-set` into its own directory without merging")
    sealed = next((i for i, x in enumerate(rel) if "needs.seal.outputs.sums" in str((x.get("env") or {}).get("SEALED", ""))), None)
    strict = at(rel, "sha256sum --strict -c SHA256SUMS")
    provenance = at(rel, "verify-provenance.sh")
    if sealed is None or strict is None or sealed != strict:
        bad("release does not compare the set with the digest the sealing job reported and check every file against the checksums")
    if provenance is None or rel[provenance].get("if") != NOT_DRY or rel[provenance].get("continue-on-error"):
        bad("release does not verify the provenance again, or can skip it in a real run")
    elif (rel[provenance].get("env") or {}).get("SOURCE_COMMIT") != COMMIT or (rel[provenance].get("env") or {}).get("SOURCE_REF") != "refs/tags/${{ needs.verify.outputs.tag }}":
        bad("release verifies the provenance for something other than the tag and the commit `verify` named")
    exact = at(rel, "exact \"$VERSION\" published")
    sigs = at(rel, "verify-signatures.sh")
    tagged = next((i for i, x in enumerate(rel) if str(x.get("name", "")).startswith("The tag still points at the commit that was built")), None)
    created = next((i for i, s in enumerate(rel) if "gh release create" in str(s.get("run", ""))), None)
    if None in (exact, sigs, tagged, created, sealed, provenance) or not exact < sealed < sigs < tagged < created or not sealed < provenance < created:
        bad("release does not check the set, the digest, the signatures, the provenance and the tag before it creates the release")
    elif "--verify-tag" not in rel[created]["run"] or "expected-assets.py" not in rel[created]["run"] or "${files[@]}" not in rel[created]["run"]:
        bad("`gh release create` does not pass --verify-tag and the files named from the list of the version")
    elif rel[created].get("if") != "${{ !inputs.dry_run }}":
        bad("`gh release create` is not skipped in a rehearsal")
    elif "*" in rel[created]["run"].replace("${files[@]}", ""):
        bad("`gh release create` names files with a glob")
    if rel[created].get("working-directory") != REL:
        bad("`gh release create` does not run in the directory of the sealed set")
    if not any("inputs.dry_run" in str(s.get("if", "")) and "Say what a rehearsal" in str(s.get("name", "")) for s in rel):
        bad("a rehearsal does not say what it would have published")

    # 6. What `attest` covers is what `verify-provenance.sh` asks for, and what it downloads is every
    # artifact of a binary or a package that the builds upload and none of the others.
    attest = jobs["attest"]["steps"]
    subjects = next(s for s in attest if uses(s, "attest-build-provenance"))["with"]["subject-path"].splitlines()
    globs = [g.strip().removeprefix(ART + "/") for g in subjects if g.strip()]
    for name in expected("binaries") + [n for n in expected("packages")]:
        if not any(fnmatch.fnmatchcase(name, g) for g in globs):
            bad(f"`attest` does not cover {name}, which the release signs and publishes as a binary or a package")
    for g in globs:
        if not g.startswith("podup"):
            bad(f"`attest` covers `{g}`, which is not a binary or a package")
    download = next(s for s in attest if uses(s, "download-artifact"))
    pattern = download["with"].get("pattern", "")
    if download["with"].get("path") != ART:
        bad("`attest` does not download into the artifact directory below runner.temp")
    check = next((i for i, s in enumerate(attest) if "check-artifact-dir.sh" in str(s.get("run", "")) and " attest" in s["run"]), None)
    attesting = next(i for i, s in enumerate(attest) if uses(s, "attest-build-provenance"))
    if check is None or check > attesting:
        bad("`attest` does not check the artifact names before it attests")

    # 7. The artifacts the builds upload, and the names the signers and the sealer read them under.
    uploads = {}
    for jname, job in rjobs.items():
        for s in job.get("steps") or []:
            if uses(s, "upload-artifact"):
                uploads.setdefault(jname, []).append(s["with"]["name"])
    def expand(job, name):
        matrix = (job.get("strategy") or {}).get("matrix") or {}
        rows = matrix.get("include") or []
        if not rows:
            return [name]
        names = []
        for row in rows:
            n = name
            for k, v in row.items():
                n = n.replace("${{ matrix.%s }}" % k, str(v))
            names.append(n)
        return names
    built = {n for jn in ("build", "build-deb", "build-rpm", "build-pacman") for n in sum((expand(rjobs[jn], u) for u in uploads.get(jn, [])), [])}
    for name in built:
        if not fnmatch.fnmatchcase(name, pattern):
            bad(f"`attest` downloads `{pattern}`, which does not match the artifact {name}")
    for name in sum((expand(rjobs[jn], u) for jn in ("sbom-tools", "sbom") for u in uploads.get(jn, [])), []):
        if fnmatch.fnmatchcase(name, pattern):
            bad(f"`attest` would download `{name}`, which is not a binary or a package")
    binaries = [i["asset"] for i in rjobs["build"]["strategy"]["matrix"]["include"]]
    if sorted(jobs["sign"]["strategy"]["matrix"]["asset"]) != sorted(binaries) or sorted(binaries) != sorted(expected("binaries")):
        bad("the signing matrix is not the set of binaries the build makes and the list of the version names")
    package_artifacts = sorted(r["artifact"] for r in jobs["sign-packages"]["strategy"]["matrix"]["include"])
    package_built = sorted(n for n in built if n not in binaries)
    if package_artifacts != package_built:
        bad(f"the package signing matrix {package_artifacts} is not the packages the builds upload {package_built}")
    files = sorted(r["file"].replace("%V", VERSION) for r in jobs["sign-packages"]["strategy"]["matrix"]["include"])
    if files != sorted(expected("packages")):
        bad("the files the package signing matrix expects are not the packages the version names")
    for jname, label in (("sign", "asset"), ("sign-packages", "artifact")):
        up = next(s for s in jobs[jname]["steps"] if uses(s, "upload-artifact"))["with"]
        if up["name"] != "sig-${{ matrix.%s }}" % label:
            bad(f"the signature artifact of `{jname}` is not named `sig-<{label}>`")
        if fnmatch.fnmatchcase(up["name"], pattern):
            bad(f"the signature artifact of `{jname}` matches what `attest` downloads")
    sig_names = ["sig-" + b for b in binaries] + ["sig-" + a for a in package_artifacts]
    allnames = sorted(built | set(sig_names) | {"sbom", "sbom-tools", "release-set"})
    if len(allnames) != len(set(allnames)) or len(built) != len(list(built)):
        bad("two artifacts would carry the same name")
    seal_down = [s["with"] for s in seal if uses(s, "download-artifact")]
    if not any(w.get("pattern") == "podup-*" for w in seal_down) or not any(w.get("pattern") == "sig-*" for w in seal_down) \
            or not any(w.get("name") == "sbom" for w in seal_down):
        bad("seal does not download the binaries and packages, the signatures and the SBOMs")
    if any(w.get("name") == "sbom-tools" or w.get("pattern") in ("*", None) and "name" not in w for w in seal_down):
        bad("seal downloads the tools of the SBOM job or everything")
    for w in seal_down:
        if "merge-multiple" in w:
            bad("seal merges artifacts on download")

    # 8. The SBOM: the tools are built in a job with no checkout and no source, and the job that
    # makes the SBOMs builds nothing.
    tools = rjobs["sbom-tools"]["steps"]
    if any(uses(s, "actions/checkout") for s in tools):
        bad("`sbom-tools` checks out the source, where a build script could write")
    sbom = rjobs["sbom"]
    if any("cargo install" in str(s.get("run", "")) for s in sbom["steps"]):
        bad("`sbom` builds a tool, which runs the build scripts of other people's crates beside the source")
    if "sbom-tools" not in needs_of(sbom):
        bad("`sbom` does not wait for the tools")
    if not any("git status --porcelain" in str(s.get("run", "")) and "expected-assets.py" in str(s.get("run", "")) for s in sbom["steps"]):
        bad("`sbom` does not check that it changed no file of the source and wrote exactly the SBOMs")
    if any(uses(s, "download-artifact") and not (s.get("with") or {}).get("name") == "sbom-tools" for s in sbom["steps"]):
        bad("`sbom` downloads something other than its tools, into a workspace that holds the source")

    # 9. Python is started with -I wherever the tag's own scripts run, and nothing is piped into a shell.
    for label, group in (("release.yml", jobs), ("the reusable", rjobs)):
        for name, job in group.items():
            out += python_without_isolation(runs(job), f"{label} `{name}`")
            for s in runs(job):
                if re.search(r"\|\s*(sudo\s+)?(ba)?sh\b", str(s["run"])):
                    bad(f"{label} `{name}`: `{s.get('name')}` pipes something into a shell")

    # 10. A rehearsal never reads the real key, and the key in a real run is the secret.
    for name in KEY_JOBS:
        for s in runs(jobs[name]):
            key = (s.get("env") or {}).get("GLYNDOR_RELEASE_ED25519_KEY")
            if key is not None and key != "${{ inputs.dry_run && env.REHEARSAL_SEED_B64 || secrets.GLYNDOR_RELEASE_ED25519_KEY }}":
                bad(f"`{name}`: the key of `{s.get('name')}` is not the secret in a real run and the throwaway key in a rehearsal")
    # 10b. A job that runs in a container is root in a workspace the runner's user owns: it trusts the workspace
    # before the first git command, or git stops on "dubious ownership", the check of the checkout included.
    for label, group in (("the builds", rjobs), ("release.yml", jobs), ("the verification", vjobs)):
        for name, job in group.items():
            if "container" not in job:
                continue
            steps = job.get("steps") or []
            trust = next((i for i, st in enumerate(steps) if "safe.directory" in str(st.get("run", ""))), None)
            first_git = next((i for i, st in enumerate(steps) if re.search(r"(^|[;&|(]|\$\(|xargs [^\n]*)\s*git\s+[a-z]", str(st.get("run", "")), re.M) and i != trust), None)
            if trust is None or (first_git is not None and first_git < trust):
                bad(f"{label} `{name}` runs in a container and uses git before it trusts the workspace")

    # 11. Nothing a release builds from or into is a cache shared with other runs: no cache action, no
    # setup action asked to cache, no restore keys, in any job of the release or of its reusables.
    for label, group in (("release.yml", jobs), ("the builds", rjobs), ("the verification", vjobs)):
        for name, job in group.items():
            for s in job.get("steps") or []:
                with_keys = {str(k) for k in (s.get("with") or {})}
                if "cache" in str(s.get("uses", "")).lower() or with_keys & CACHE_INPUTS:
                    bad(f"{label} `{name}`: `{s.get('name') or s.get('uses')}` restores or saves a cache, which another run can poison")
    if "dry_run" not in (spec.get("on", spec.get(True)) or {}).get("workflow_dispatch", {}).get("inputs", {}):
        bad("release.yml has no `dry_run` input")
    return out


if __name__ == "__main__":
    found = problems(sys.argv[1] if len(sys.argv) > 1 else f"{REPO}/.github/workflows/release.yml")
    print("\n".join(found))
    sys.exit(1 if found else 0)
