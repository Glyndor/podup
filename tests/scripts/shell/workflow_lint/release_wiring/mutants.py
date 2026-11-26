#!/usr/bin/env python3
"""Mutate release.yml (and the reusable of the builds) one wiring fact at a time; each must be reported.

Usage: mutants.py   (prints `killed N`, `SURVIVED name` per survivor)
"""
import copy
import os
import shutil
import sys
import tempfile

import yaml

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import wiring  # noqa: E402

DIR = f"{wiring.REPO}/.github/workflows"
spec = yaml.safe_load(open(f"{DIR}/release.yml"))
reusable = yaml.safe_load(open(f"{DIR}/reusable-release-build.yml"))
verifying = yaml.safe_load(open(f"{DIR}/reusable-release-verify.yml"))
ART = wiring.ART


def step(m, job, needle):
    return next(s for s in m["jobs"][job]["steps"] if needle in str(s.get("name", "")) or needle in str(s.get("uses", "")) or needle in str(s.get("run", "")))


def drop_line(s, needle):
    s["run"] = "\n".join(l for l in s["run"].splitlines() if needle not in l)


MUTANTS = {}  # name -> (function over (release, builds, verification))


def mutant(fn):
    MUTANTS[fn.__name__] = fn
    return fn


def rel(fn):
    return lambda r, b, v: fn(r)


def bld(fn):
    return lambda r, b, v: fn(b)


def ver(fn):
    return lambda r, b, v: fn(v)


# --- permissions and who can mint ---------------------------------------------------------------
for _j in ("verify", "builds", "sign", "sign-packages", "seal", "release"):
    MUTANTS[f"id-token-on-{_j}"] = rel((lambda j: lambda m: m["jobs"][j]["permissions"].update({"id-token": "write"}))(_j))
for _j in ("builds", "sign", "sign-packages", "seal", "release"):
    MUTANTS[f"attestations-write-on-{_j}"] = rel((lambda j: lambda m: m["jobs"][j]["permissions"].update({"attestations": "write"}))(_j))
MUTANTS["attest-without-id-token"] = rel(lambda m: m["jobs"]["attest"]["permissions"].pop("id-token"))
MUTANTS["release-contents-read"] = rel(lambda m: m["jobs"]["release"]["permissions"].update({"contents": "read"}))
MUTANTS["seal-may-write"] = rel(lambda m: m["jobs"]["seal"]["permissions"].update({"contents": "write"}))
MUTANTS["sign-may-write"] = rel(lambda m: m["jobs"]["sign"]["permissions"].update({"contents": "write"}))
MUTANTS["a-job-of-the-reusable-may-write"] = bld(lambda m: m["jobs"]["build"].update({"permissions": {"contents": "write"}}))
MUTANTS["the-reusable-may-write-at-the-top"] = bld(lambda m: m.update({"permissions": {"contents": "write"}}))
MUTANTS["a-job-of-the-reusable-has-an-environment"] = bld(lambda m: m["jobs"]["sbom"].update({"environment": "release"}))
for _j in ("sign", "sign-packages", "seal"):
    MUTANTS[f"{_j}-without-the-environment"] = rel((lambda j: lambda m: m["jobs"][j].pop("environment"))(_j))
MUTANTS["release-has-the-environment"] = rel(lambda m: m["jobs"]["release"].update({"environment": "release"}))
MUTANTS["release-reaches-for-the-key"] = rel(lambda m: step(m, "release", "Create GitHub Release").setdefault("env", {}).update(
    {"K": "${{ secrets.GLYNDOR_RELEASE_ED25519_KEY }}"}))

# --- the graph ------------------------------------------------------------------------------------
MUTANTS["seal-without-sign"] = rel(lambda m: m["jobs"]["seal"]["needs"].remove("sign"))
MUTANTS["seal-without-sign-packages"] = rel(lambda m: m["jobs"]["seal"]["needs"].remove("sign-packages"))
MUTANTS["seal-without-attest"] = rel(lambda m: m["jobs"]["seal"]["needs"].remove("attest"))
MUTANTS["release-without-seal"] = rel(lambda m: m["jobs"]["release"].update({"needs": ["verify"]}))
MUTANTS["sign-without-verify"] = rel(lambda m: m["jobs"]["sign"].update({"needs": ["builds"]}))
MUTANTS["builds-is-another-workflow"] = rel(lambda m: m["jobs"]["builds"].update({"uses": "./.github/workflows/other.yml"}))
MUTANTS["builds-without-the-tag"] = rel(lambda m: m["jobs"]["builds"]["with"].pop("tag"))
MUTANTS["builds-without-the-commit"] = rel(lambda m: m["jobs"]["builds"]["with"].pop("commit"))
MUTANTS["builds-with-another-version"] = rel(lambda m: m["jobs"]["builds"]["with"].update({"version": "${{ inputs.tag }}"}))
MUTANTS["verify-outputs-nothing"] = ver(lambda m: m["jobs"]["verify"].pop("outputs"))
MUTANTS["verification-outputs-lost-the-tag"] = ver(lambda m: m["on" if "on" in m else True]["workflow_call"]["outputs"].pop("tag"))
MUTANTS["the-audit-has-outputs"] = ver(lambda m: m["jobs"]["audit"].update({"outputs": {"x": "y"}}))
MUTANTS["verify-installs-a-tool"] = ver(lambda m: m["jobs"]["verify"]["steps"].append({"name": "x", "run": "cargo install cargo-audit"}))
MUTANTS["verify-has-no-pin-step"] = ver(lambda m: step(m, "verify", "Name the commit").pop("id"))
MUTANTS["the-published-check-has-a-condition"] = ver(lambda m: step(m, "verify", "Refuse an already published").update({"if": "${{ !inputs.dry_run }}"}))
MUTANTS["a-published-release-is-refused-in-a-rehearsal"] = ver(lambda m: step(m, "verify", "Refuse an already published")["env"].update({"MODE": "release"}))
MUTANTS["a-published-release-is-accepted-in-a-release"] = ver(lambda m: step(m, "verify", "Refuse an already published")["env"].update({"MODE": "rehearsal"}))
MUTANTS["the-published-check-is-inline-again"] = ver(lambda m: step(m, "verify", "Refuse an already published").update({"run": "gh api repos/x/releases"}))
MUTANTS["builds-without-dry-run"] = rel(lambda m: m["jobs"]["builds"]["with"].pop("dry_run"))
MUTANTS["the-tag-is-not-asked-whether-it-is-on-main"] = ver(lambda m: m["jobs"]["verify"]["steps"].remove(step(m, "verify", "Tag must be on main")))
MUTANTS["the-main-check-is-conditional"] = ver(lambda m: step(m, "verify", "Tag must be on main").update({"if": "${{ !inputs.dry_run }}"}))
MUTANTS["the-main-check-may-fail"] = ver(lambda m: step(m, "verify", "Tag must be on main").update({"continue-on-error": True}))
MUTANTS["the-main-check-asks-the-tag"] = ver(lambda m: step(m, "verify", "Tag must be on main").update({"run": "bash .github/scripts/release/check-tag.sh main"}))
MUTANTS["the-main-check-names-no-branch"] = ver(lambda m: step(m, "verify", "Tag must be on main").update({"run": step(m, "verify", "Tag must be on main")["run"].replace('"+refs/heads/main:refs/remotes/origin/main"', "main")}))
MUTANTS["the-main-check-after-the-scripts-of-the-tag"] = ver(lambda m: m["jobs"]["verify"]["steps"].insert(next(i for i, x in enumerate(m["jobs"]["verify"]["steps"]) if x.get("name") == "Tag must be an existing tag object at the checked-out commit") + 1, m["jobs"]["verify"]["steps"].pop(next(i for i, x in enumerate(m["jobs"]["verify"]["steps"]) if x.get("name") == "Tag must be on main"))))
MUTANTS["verify-checks-out-another-tag"] = ver(lambda m: step(m, "verify", "actions/checkout")["with"].update({"ref": "refs/heads/main"}))
MUTANTS["the-audit-checks-out-a-revision-an-output-carries"] = ver(lambda m: step(m, "audit", "actions/checkout")["with"].update({"ref": "${{ needs.verify.outputs.commit }}"}))
MUTANTS["verify-called-with-another-tag"] = rel(lambda m: m["jobs"]["verify"]["with"].update({"tag": "${{ github.ref_name }}"}))
MUTANTS["verify-is-another-workflow"] = rel(lambda m: m["jobs"]["verify"].update({"uses": "./.github/workflows/reusable-release-build.yml"}))

# --- the commit -----------------------------------------------------------------------------------
for _j in ("sign", "sign-packages", "attest", "seal", "release"):
    MUTANTS[f"{_j}-checks-out-a-revision-an-output-carries"] = rel((lambda j: lambda m: step(m, j, "actions/checkout")["with"].update(
        {"ref": "${{ needs.verify.outputs.commit }}"}))(_j))
    MUTANTS[f"{_j}-checks-out-another-tag"] = rel((lambda j: lambda m: step(m, j, "actions/checkout")["with"].update(
        {"ref": "refs/tags/${{ needs.verify.outputs.tag }}"}))(_j))
    MUTANTS[f"{_j}-does-not-check-the-checkout"] = rel((lambda j: lambda m: m["jobs"][j]["steps"].remove(step(m, j, "The checkout is the commit")))(_j))
for _j in ("build", "build-deb", "build-rpm", "build-pacman", "sbom"):
    MUTANTS[f"reusable-{_j}-checks-out-another-tag"] = bld((lambda j: lambda m: step(m, j, "actions/checkout")["with"].update(
        {"ref": "refs/tags/v1.0.0"}))(_j))
    MUTANTS[f"reusable-{_j}-checks-out-a-revision-an-input-carries"] = bld((lambda j: lambda m: step(m, j, "actions/checkout")["with"].update(
        {"ref": "${{ inputs.commit }}"}))(_j))
    MUTANTS[f"reusable-{_j}-does-not-check-the-checkout"] = bld((lambda j: lambda m: m["jobs"][j]["steps"].remove(step(m, j, "The checkout is the commit")))(_j))

# --- the sealing job ------------------------------------------------------------------------------
MUTANTS["seal-without-the-staged-check"] = rel(lambda m: drop_line(step(m, "seal", "Gather the artifacts"), "staged"))
MUTANTS["seal-without-the-final-check"] = rel(lambda m: drop_line(step(m, "seal", "Verify every signature"), "published"))
MUTANTS["provenance-can-be-skipped"] = rel(lambda m: step(m, "seal", "Verify the provenance").update({"if": "false"}))
MUTANTS["provenance-for-another-ref"] = rel(lambda m: step(m, "seal", "Verify the provenance")["env"].update({"SOURCE_REF": "refs/heads/main"}))
MUTANTS["provenance-for-another-commit"] = rel(lambda m: step(m, "seal", "Verify the provenance")["env"].update({"SOURCE_COMMIT": "${{ github.sha }}"}))
MUTANTS["staged-check-can-be-skipped"] = rel(lambda m: step(m, "seal", "Gather the artifacts").update({"if": "false"}))
MUTANTS["seal-merges-on-download"] = rel(lambda m: next(s for s in m["jobs"]["seal"]["steps"] if "download-artifact" in str(s.get("uses", "")))["with"].update({"merge-multiple": True}))
MUTANTS["seal-downloads-everything"] = rel(lambda m: next(s for s in m["jobs"]["seal"]["steps"] if "download-artifact" in str(s.get("uses", "")))["with"].pop("pattern"))
MUTANTS["seal-uploads-another-name"] = rel(lambda m: step(m, "seal", "upload-artifact")["with"].update({"name": "set"}))
MUTANTS["seal-uploads-another-path"] = rel(lambda m: step(m, "seal", "upload-artifact")["with"].update({"path": "${{ runner.temp }}/downloaded"}))
for _name in ("Verify the Linux", "Generate checksums", "Sign SHA256SUMS", "Verify every signature", "Verify every file the installers"):
    MUTANTS[f"seal-no-working-directory-{_name.split()[1]}"] = rel((lambda n: lambda m: step(m, "seal", n).pop("working-directory"))(_name))
MUTANTS["checksums-from-a-glob"] = rel(lambda m: step(m, "seal", "Generate checksums").update(
    {"run": "sha256sum podup-* > SHA256SUMS"}))
MUTANTS["the-key-in-a-real-run-is-not-the-secret"] = rel(lambda m: step(m, "seal", "Sign SHA256SUMS")["env"].update(
    {"GLYNDOR_RELEASE_ED25519_KEY": "${{ env.REHEARSAL_SEED_B64 }}"}))
MUTANTS["a-rehearsal-reads-the-real-key"] = rel(lambda m: step(m, "sign", "Sign binary")["env"].update(
    {"GLYNDOR_RELEASE_ED25519_KEY": "${{ secrets.GLYNDOR_RELEASE_ED25519_KEY }}"}))

# --- the publishing job ---------------------------------------------------------------------------
MUTANTS["no-verify-tag"] = rel(lambda m: drop_line(step(m, "release", "Create GitHub Release"), "--verify-tag"))
MUTANTS["create-names-by-a-glob"] = rel(lambda m: step(m, "release", "Create GitHub Release").update(
    {"run": 'gh release create "$TAG" --verify-tag ./*  # expected-assets.py ${files[@]}'}))
MUTANTS["a-rehearsal-publishes"] = rel(lambda m: step(m, "release", "Create GitHub Release").pop("if"))
MUTANTS["a-rehearsal-says-nothing"] = rel(lambda m: m["jobs"]["release"]["steps"].remove(step(m, "release", "Say what a rehearsal")))
MUTANTS["create-not-in-the-release-directory"] = rel(lambda m: step(m, "release", "Create GitHub Release").pop("working-directory"))
MUTANTS["release-merges-on-download"] = rel(lambda m: step(m, "release", "download-artifact")["with"].update({"merge-multiple": True}))
MUTANTS["release-downloads-another-artifact"] = rel(lambda m: step(m, "release", "download-artifact")["with"].update({"name": "sbom"}))
MUTANTS["release-without-the-set-check"] = rel(lambda m: m["jobs"]["release"]["steps"].remove(step(m, "release", "The files are exactly")))
MUTANTS["release-without-the-signature-check"] = rel(lambda m: m["jobs"]["release"]["steps"].remove(step(m, "release", "Verify every signature")))
MUTANTS["release-without-the-tag-check"] = rel(lambda m: m["jobs"]["release"]["steps"].remove(step(m, "release", "The tag still points")))
MUTANTS["tag-check-after-create"] = rel(lambda m: m["jobs"]["release"]["steps"].append(m["jobs"]["release"]["steps"].pop(
    m["jobs"]["release"]["steps"].index(step(m, "release", "The tag still points")))))
MUTANTS["no-dry-run-input"] = rel(lambda m: m["on" if "on" in m else True]["workflow_dispatch"]["inputs"].pop("dry_run"))

# --- what attest covers ---------------------------------------------------------------------------
def narrow_subject(index):
    def fn(m):
        s = step(m, "attest", "attest-build-provenance")
        lines = s["with"]["subject-path"].splitlines()
        del lines[index]
        s["with"]["subject-path"] = "\n".join(lines) + "\n"
    return fn


for _i in range(6):
    MUTANTS[f"attest-subject-{_i}-missing"] = rel(narrow_subject(_i))
MUTANTS["attest-subject-only-linux-x86"] = rel(lambda m: step(m, "attest", "attest-build-provenance")["with"].update(
    {"subject-path": f"{ART}/podup-linux-x86_64\n{ART}/podup-linux-arm64\n"}))
MUTANTS["attest-covers-the-signatures"] = rel(lambda m: step(m, "attest", "attest-build-provenance")["with"].update(
    {"subject-path": step(m, "attest", "attest-build-provenance")["with"]["subject-path"] + f"{ART}/sig-*\n"}))
MUTANTS["attest-pattern-narrower"] = rel(lambda m: step(m, "attest", "download-artifact")["with"].update({"pattern": "podup-linux-*"}))
MUTANTS["attest-pattern-wider"] = rel(lambda m: step(m, "attest", "download-artifact")["with"].update({"pattern": "*"}))
MUTANTS["attest-downloads-into-the-workspace"] = rel(lambda m: step(m, "attest", "download-artifact")["with"].update({"path": "${{ github.workspace }}"}))
MUTANTS["attest-without-the-name-check"] = rel(lambda m: m["jobs"]["attest"]["steps"].remove(step(m, "attest", "Check the artifacts")))
MUTANTS["attest-checks-the-names-after-attesting"] = rel(lambda m: m["jobs"]["attest"]["steps"].append(
    m["jobs"]["attest"]["steps"].pop(m["jobs"]["attest"]["steps"].index(step(m, "attest", "Check the artifacts")))))

# --- the signing matrices -------------------------------------------------------------------------
MUTANTS["sign-matrix-short"] = rel(lambda m: m["jobs"]["sign"]["strategy"]["matrix"]["asset"].pop())
MUTANTS["sign-packages-matrix-short"] = rel(lambda m: m["jobs"]["sign-packages"]["strategy"]["matrix"]["include"].pop())
MUTANTS["sign-packages-another-file"] = rel(lambda m: m["jobs"]["sign-packages"]["strategy"]["matrix"]["include"][2].update({"file": "podup-%V.rpm"}))
MUTANTS["sig-artifact-name-collides"] = rel(lambda m: step(m, "sign", "upload-artifact")["with"].update({"name": "${{ matrix.asset }}"}))
MUTANTS["package-sig-artifact-name-collides"] = rel(lambda m: step(m, "sign-packages", "upload-artifact")["with"].update({"name": "${{ matrix.artifact }}"}))
MUTANTS["deb-artifact-name-collides"] = bld(lambda m: step(m, "build-deb", "upload-artifact")["with"].update({"name": "podup-deb"}))
MUTANTS["rpm-artifact-is-not-attested"] = bld(lambda m: step(m, "build-rpm", "upload-artifact")["with"].update({"name": "rpm-${{ matrix.rpm_arch }}"}))
MUTANTS["sbom-artifact-is-attested"] = bld(lambda m: step(m, "sbom", "upload-artifact")["with"].update({"name": "podup-sbom"}))
MUTANTS["sign-matrix-lost-a-build"] = bld(lambda m: m["jobs"]["build"]["strategy"]["matrix"]["include"].pop())

# --- the SBOM -------------------------------------------------------------------------------------
MUTANTS["tools-built-beside-the-source"] = bld(lambda m: m["jobs"]["sbom-tools"]["steps"].insert(0, {"uses": "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1", "with": {"persist-credentials": False}}))
MUTANTS["sbom-builds-a-tool"] = bld(lambda m: m["jobs"]["sbom"]["steps"].append({"name": "x", "run": "cargo install --locked cargo-about"}))
MUTANTS["sbom-does-not-wait-for-the-tools"] = bld(lambda m: m["jobs"]["sbom"].update({"needs": []}))
MUTANTS["sbom-does-not-check-the-source"] = bld(lambda m: m["jobs"]["sbom"]["steps"].remove(step(m, "sbom", "Nothing of the source changed")))
MUTANTS["sbom-downloads-the-built-files"] = bld(lambda m: m["jobs"]["sbom"]["steps"].insert(0, {"uses": "actions/download-artifact@9000827ccba6bdab643e8b6fd33ac0654aef8333", "with": {"merge-multiple": True}}))

# --- Python and the shell -------------------------------------------------------------------------
def without_isolation(job_group, job, needle):
    def fn(r, b, v):
        m = r if job_group == "r" else b
        s = step(m, job, needle)
        s["run"] = s["run"].replace("python3 -I ", "python3 ").replace('-I -X utf8', '-X utf8').replace('"$venv_py" -I ', '"$venv_py" ')
    return fn


MUTANTS["sign-python-without-isolation"] = without_isolation("r", "sign", "Sign binary")
MUTANTS["sign-packages-python-without-isolation"] = without_isolation("r", "sign-packages", "Sign package")
MUTANTS["seal-checksums-python-without-isolation"] = without_isolation("r", "seal", "Generate checksums")
MUTANTS["seal-signing-python-without-isolation"] = without_isolation("r", "seal", "Sign SHA256SUMS")
MUTANTS["seal-assets-python-without-isolation"] = without_isolation("r", "seal", "Verify every file the installers")
MUTANTS["release-verify-python-without-isolation"] = without_isolation("r", "release", "Verify every signature")
MUTANTS["release-create-python-without-isolation"] = without_isolation("r", "release", "Create GitHub Release")
MUTANTS["stage-python-without-isolation"] = without_isolation("b", "build-rpm", "Stage the payload")
for _j in ("build-rpm", "build-pacman"):
    MUTANTS[f"{_j}-stages-without-the-epoch"] = (lambda j: lambda r, b, v: step(b, j, "Stage the payload").update(
        {"run": step(b, j, "Stage the payload")["run"].replace("export SOURCE_DATE_EPOCH\n", "")}))(_j)
MUTANTS["rustup-piped-into-a-shell"] = bld(lambda m: step(m, "build-deb", "Install the pinned Rust").update(
    {"run": "curl -fsSL https://sh.rustup.rs | sh -s -- -y"}))

# --- the tag is looked up again, and the provenance is checked before anything is signed ---------
for _j in ("sign", "sign-packages", "attest", "seal", "release"):
    MUTANTS[f"{_j}-without-the-tag-lookup"] = rel((lambda j: lambda m: m["jobs"][j]["steps"].remove(step(m, j, "The tag still points at the commit that was verified")))(_j))
    MUTANTS[f"{_j}-looks-the-tag-up-after-a-script"] = rel((lambda j: lambda m: m["jobs"][j]["steps"].append(m["jobs"][j]["steps"].pop(
        m["jobs"][j]["steps"].index(step(m, j, "The tag still points at the commit that was verified")))))(_j))
    MUTANTS[f"{_j}-compares-the-tag-with-another-commit"] = rel((lambda j: lambda m: step(m, j, "The tag still points at the commit that was verified")["env"].update(
        {"COMMIT": "${{ github.sha }}"}))(_j))
MUTANTS["attest-runs-in-a-rehearsal"] = rel(lambda m: m["jobs"]["attest"].pop("if"))
MUTANTS["attest-skipped-always"] = rel(lambda m: m["jobs"]["attest"].update({"if": "false"}))
for _j in ("sign", "sign-packages", "seal"):
    MUTANTS[f"{_j}-does-not-wait-for-attest"] = rel((lambda j: lambda m: m["jobs"][j]["needs"].remove("attest"))(_j))
    MUTANTS[f"{_j}-accepts-any-skip"] = rel((lambda j: lambda m: m["jobs"][j].update({"if": "${{ !cancelled() }}"}))(_j))
    MUTANTS[f"{_j}-without-a-condition"] = rel((lambda j: lambda m: m["jobs"][j].pop("if"))(_j))
    MUTANTS[f"{_j}-provenance-is-skippable"] = rel((lambda j: lambda m: step(m, j, "Verify the provenance").update({"if": "false"}))(_j))
    MUTANTS[f"{_j}-provenance-without-its-condition"] = rel((lambda j: lambda m: step(m, j, "Verify the provenance").pop("if"))(_j))
    MUTANTS[f"{_j}-provenance-may-fail"] = rel((lambda j: lambda m: step(m, j, "Verify the provenance").update({"continue-on-error": True}))(_j))
    MUTANTS[f"{_j}-provenance-after-signing"] = rel((lambda j: lambda m: m["jobs"][j]["steps"].append(m["jobs"][j]["steps"].pop(
        m["jobs"][j]["steps"].index(step(m, j, "Verify the provenance")))))(_j))
    MUTANTS[f"{_j}-provenance-for-another-commit"] = rel((lambda j: lambda m: step(m, j, "Verify the provenance")["env"].update({"SOURCE_COMMIT": "${{ github.sha }}"}))(_j))
for _j in ("sign", "sign-packages"):
    MUTANTS[f"{_j}-verifies-no-file"] = rel((lambda j: lambda m: step(m, j, "Verify the provenance")["env"].pop("NAME"))(_j))
    MUTANTS[f"{_j}-has-no-attestations-read"] = rel((lambda j: lambda m: m["jobs"][j]["permissions"].pop("attestations"))(_j))
MUTANTS["seal-reports-no-digest"] = rel(lambda m: m["jobs"]["seal"].pop("outputs"))
MUTANTS["seal-reports-another-digest"] = rel(lambda m: m["jobs"]["seal"]["outputs"].update({"sums": "${{ github.sha }}"}))
MUTANTS["release-without-the-digest-step"] = rel(lambda m: m["jobs"]["release"]["steps"].remove(step(m, "release", "The set is the one the sealing job made")))
MUTANTS["release-compares-with-nothing"] = rel(lambda m: step(m, "release", "The set is the one the sealing job made")["env"].update({"SEALED": "${{ github.sha }}"}))
MUTANTS["release-does-not-check-every-file"] = rel(lambda m: drop_line(step(m, "release", "The set is the one the sealing job made"), "--strict -c"))
MUTANTS["release-without-the-provenance"] = rel(lambda m: m["jobs"]["release"]["steps"].remove(step(m, "release", "Verify the provenance of the binaries")))
MUTANTS["release-provenance-skippable"] = rel(lambda m: step(m, "release", "Verify the provenance of the binaries").update({"if": "false"}))
MUTANTS["release-provenance-for-another-ref"] = rel(lambda m: step(m, "release", "Verify the provenance of the binaries")["env"].update({"SOURCE_REF": "refs/heads/main"}))
MUTANTS["release-provenance-after-create"] = rel(lambda m: m["jobs"]["release"]["steps"].append(m["jobs"]["release"]["steps"].pop(
    m["jobs"]["release"]["steps"].index(step(m, "release", "Verify the provenance of the binaries")))))
MUTANTS["release-cannot-read-attestations"] = rel(lambda m: m["jobs"]["release"]["permissions"].pop("attestations"))

# --- git in the container ---------------------------------------------------------------------------
MUTANTS["the-container-job-does-not-trust-the-workspace"] = bld(lambda m: m["jobs"]["build-deb"]["steps"].remove(step(m, "build-deb", "Trust the workspace")))
MUTANTS["the-container-job-trusts-the-workspace-too-late"] = bld(lambda m: m["jobs"]["build-deb"]["steps"].append(
    m["jobs"]["build-deb"]["steps"].pop(m["jobs"]["build-deb"]["steps"].index(step(m, "build-deb", "Trust the workspace")))))

# --- no shared cache ------------------------------------------------------------------------------
def sbom_call(m, old, new):
    s = step(m, "sbom", "gen_sbom")
    assert old in s["run"]
    s["run"] = s["run"].replace(old, new, 1)


MUTANTS["the-deb-sbom-is-made-for-gnu"] = bld(lambda m: sbom_call(m, 'amd64.deb" x86_64-unknown-linux-musl', 'amd64.deb" x86_64-unknown-linux-gnu'))
MUTANTS["the-deb-sbom-loses-the-features"] = bld(lambda m: sbom_call(m, "--features watch,completions", "--features watch"))
MUTANTS["the-deb-sbom-of-arm64-is-not-made"] = bld(lambda m: sbom_call(m, 'gen_sbom "podup_${VERSION}_arm64.deb"', ': "podup_${VERSION}_arm64.deb"'))
CACHE = {"name": "cache", "uses": "actions/cache@0000000000000000000000000000000000000000", "with": {"path": "target", "key": "k"}}
for _j in ("build", "build-deb", "build-rpm", "build-pacman", "sbom"):
    MUTANTS[f"reusable-{_j}-restores-a-cache"] = bld((lambda j: lambda m: m["jobs"][j]["steps"].insert(1, dict(CACHE)))(_j))
MUTANTS["the-build-uses-a-rust-cache"] = bld(lambda m: m["jobs"]["build"]["steps"].insert(1, {"name": "c", "uses": "Swatinem/rust-cache@0000000000000000000000000000000000000000"}))
MUTANTS["the-audit-restores-a-cache"] = ver(lambda m: m["jobs"]["audit"]["steps"].insert(1, dict(CACHE)))
MUTANTS["the-verification-asks-a-setup-action-to-cache"] = ver(lambda m: m["jobs"]["verify"]["steps"].insert(1, {"name": "p", "uses": "actions/setup-python@0000000000000000000000000000000000000000", "with": {"cache": "pip"}}))
MUTANTS["the-rust-download-has-restore-keys"] = bld(lambda m: step(m, "sbom", "actions/download-artifact").setdefault("with", {}).update({"restore-keys": "x"}))
for _j in ("sign", "sign-packages", "attest", "seal", "release"):
    MUTANTS[f"{_j}-restores-a-cache"] = rel((lambda j: lambda m: m["jobs"][j]["steps"].insert(1, dict(CACHE)))(_j))

dead, alive = 0, []
for name, fn in MUTANTS.items():
    r, b, v = copy.deepcopy(spec), copy.deepcopy(reusable), copy.deepcopy(verifying)
    try:
        fn(r, b, v)
    except (KeyError, StopIteration, ValueError, IndexError) as e:
        print(f"BROKEN {name}: the mutation could not be applied ({type(e).__name__}: {e})")
        alive.append(name)
        continue
    with tempfile.TemporaryDirectory() as tmp:
        with open(f"{tmp}/release.yml", "w") as fh:
            yaml.safe_dump(r, fh)
        with open(f"{tmp}/reusable-release-build.yml", "w") as fh:
            yaml.safe_dump(b, fh)
        with open(f"{tmp}/reusable-release-verify.yml", "w") as fh:
            yaml.safe_dump(v, fh)
        if wiring.problems(f"{tmp}/release.yml"):
            dead += 1
        else:
            alive.append(name)
print(f"killed {dead}")
for name in alive:
    print(f"SURVIVED {name}")
sys.exit(1 if alive else 0)
