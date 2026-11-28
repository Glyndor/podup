#!/usr/bin/env bash
#
# Behaviour tests for what the release workflow gives each job, and for the
# scripts it runs before building and before signing.
#
# A job that compiles dependencies runs code from outside this organisation
# beside whatever the job was granted. These tests fail when such a job is
# granted the permissions that mint a token (`id-token`, `attestations`), when
# the release stops naming the tag it builds or checking it first, when the
# artifacts are merged on download so one can replace another, and when the
# provenance of what gets signed is no longer verified.
#
# Everything here is static or runs the scripts as they ship against fixtures;
# what GitHub does with the workflow is not exercised.
#
# Requires: python3 with PyYAML, git.
# shellcheck disable=SC2016
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
RELEASE="$HERE/.github/workflows/release.yml"
BUILDS="$HERE/.github/workflows/reusable-release-build.yml"
VERIFYING="$HERE/.github/workflows/reusable-release-verify.yml"
REHEARSING="$HERE/.github/workflows/reusable-release-rehearsal.yml"
CHECK_DIR="$HERE/.github/scripts/release/check-artifact-dir.sh"
CHECK_TAG="$HERE/.github/scripts/release/check-tag.sh"
VERIFY_PROV="$HERE/.github/scripts/release/verify-provenance.sh"
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

# --- the shipped workflows, read as data -------------------------------------
facts="$(python3 - "$RELEASE" "$BUILDS" "$VERIFYING" "$REHEARSING" <<'PY'
import sys, yaml
jobs = yaml.safe_load(open(sys.argv[1]))["jobs"]
builds = yaml.safe_load(open(sys.argv[2]))["jobs"]
verifying = yaml.safe_load(open(sys.argv[3]))["jobs"]
rehearsing = yaml.safe_load(open(sys.argv[4]))["jobs"]

def text(job):
    return yaml.safe_dump(job)

out = {}
groups = [("release", jobs), ("builds", builds), ("verification", verifying), ("rehearsal", rehearsing)]
checkouts = [(g, n, s) for g, grp in groups for n, j in grp.items() for s in j.get("steps", [])
             if str(s.get("uses", "")).startswith("actions/checkout")]
TAG_REFS = ("refs/tags/${{ github.event_name == 'workflow_dispatch' && inputs.tag || github.ref_name }}", "refs/tags/${{ inputs.tag }}")
out["every checkout is the tag of the run"] = all(s["with"]["ref"] in TAG_REFS for g, n, s in checkouts)
out["no checkout takes a revision from an input or an output"] = not any(
    "commit" in str(s["with"]["ref"]) or not str(s["with"]["ref"]).startswith("refs/tags/") for g, n, s in checkouts)
out["verify checkout keeps no credential"] = next(
    s for s in verifying["verify"]["steps"] if str(s.get("uses", "")).startswith("actions/checkout")
)["with"].get("persist-credentials") is False
tag = next(i for i, s in enumerate(verifying["verify"]["steps"]) if "check-tag.sh" in str(s.get("run", "")))
pin = next(i for i, s in enumerate(verifying["verify"]["steps"]) if s.get("id") == "pin")
out["the tag is checked and the commit is read in a job that installs nothing"] = not any("cargo install" in str(s.get("run", "")) for s in verifying["verify"]["steps"])
out["the audit installs its tool in a job of its own with no outputs"] = any("cargo install" in str(s.get("run", "")) for s in verifying["audit"]["steps"]) and not verifying["audit"].get("outputs")
for name in ("build", "build-deb", "build-rpm", "build-pacman", "sbom", "sbom-tools"):
    out[f"{name} holds no minting permission"] = not (set(builds[name].get("permissions", {})) & {"id-token", "attestations"})
out["no attestation step in the builds"] = not any("attest-build-provenance" in text(j) for j in builds.values())
out["the builds never see a secret"] = "secrets" not in text(builds)
a = jobs["attest"]
out["attest compiles nothing"] = "cargo" not in text(a)
out["attest waits for the builds"] = "builds" in a["needs"]
out["attest checks the names before attesting"] = [
    i for i, s in enumerate(a["steps"]) if "check-artifact-dir.sh" in str(s.get("run", ""))
][0] < [i for i, s in enumerate(a["steps"]) if "attest-build-provenance" in str(s.get("uses", ""))][0]
seal = jobs["seal"]
out["seal waits for attest and both signers"] = {"attest", "sign", "sign-packages"} <= set(seal["needs"])
out["seal does not merge on download"] = all("merge-multiple" not in s.get("with", {}) for s in seal["steps"] if "download-artifact" in str(s.get("uses", "")))
steps = seal["steps"]
def at(needle):
    return next(i for i, s in enumerate(steps) if needle in str(s.get("run", "")))
out["seal gathers with duplicate detection"] = "gather" in str(steps[at("check-artifact-dir.sh")]["run"])
out["provenance is verified before signing"] = at("verify-provenance.sh") < at("sign.py")
out["the provenance step is skipped only in a rehearsal"] = steps[at("verify-provenance.sh")].get("if") == "${{ !inputs.dry_run }}" and not steps[at("verify-provenance.sh")].get("continue-on-error")
out["seal may read attestations"] = seal["permissions"].get("attestations") == "read"
out["the signature artifacts do not match podup-*"] = all(
    not s["with"]["name"].startswith("podup-")
    for j in ("sign", "sign-packages") for s in jobs[j]["steps"] if "upload-artifact" in str(s.get("uses", "")))
rel = jobs["release"]
out["release waits for seal and not for the signers alone"] = "seal" in rel["needs"]
out["release has the right to write, may read attestations, and has no key"] = rel["permissions"] == {"contents": "write", "attestations": "read"} and "secrets.GLYNDOR" not in text(rel)
out["the publishing job is in its own environment, which is not the one of the key, and not in a rehearsal"] = (
    rel.get("environment") == "release-publish" and rel.get("if") == "${{ !inputs.dry_run }}")
both = [jobs["rehearsal"], rehearsing["rehearsal"]]
out["the rehearsal can only read and is in no environment"] = all(
    j.get("permissions") == {"contents": "read"} and "environment" not in j for j in both)
out["the rehearsal never sees a secret or the token"] = not any(w in text(j) for j in both for w in ("secrets", "github.token"))
out["only the publishing job has a command that creates a release"] = [
    n for grp in (jobs, builds, verifying, rehearsing) for n, j in grp.items()
    if any("gh release create" in str(s.get("run", "")) for s in j.get("steps", []))] == ["release"]
out["no job has the key and the right to write"] = not any(
    "secrets.GLYNDOR" in text(j) and "write" in (j.get("permissions") or {}).values() for j in jobs.values())
for k, v in out.items():
    print(("1 " if v else "0 ") + k)
PY
)"
while IFS= read -r line; do
	check "${line#? }" 1 "${line%% *}"
done <<<"$facts"

# --- check-tag.sh against real repositories --------------------------------
mkrepo() { # <name>: an origin with an annotated tag v1 on its first commit
	local o="$WORK/$1-origin" c="$WORK/$1-clone"
	git init -q --bare -b main "$o"
	git init -q -b main "$WORK/$1-src"
	(
		cd "$WORK/$1-src" || exit 1
		git config user.name t && git config user.email t@example.invalid && git config commit.gpgsign false
		git commit -q --allow-empty -m one
		git -c tag.gpgSign=false tag -a v1 -m v1
		git push -q "$o" HEAD:refs/heads/main --tags
	)
	git clone -q "$o" "$c"
	(cd "$c" && git config user.name t && git config user.email t@example.invalid)
	echo "$c"
}
tag_status() { (cd "$1" && bash "$CHECK_TAG" "$2" >"$WORK/out" 2>&1; echo $?); }

c="$(mkrepo ok)"
check "an annotated tag at the checked-out commit passes" 0 "$(tag_status "$c" v1)"
c="$(mkrepo missing)"
check "a tag that does not exist stops the release" 1 "$(tag_status "$c" v9)"

c="$(mkrepo light)"
(cd "$WORK/light-src" && git commit -q --allow-empty -m two && git -c tag.gpgSign=false tag v2 && git push -q "$WORK/light-origin" HEAD:refs/heads/main v2)
(cd "$c" && git pull -q origin main && git fetch -q origin "refs/tags/*:refs/tags/*" && git checkout -q v2)
check "a lightweight tag is refused" 1 "$(tag_status "$c" v2)"
check "...saying it is not an annotated tag object" 1 "$(grep -c 'not an annotated tag object' "$WORK/out")"

c="$(mkrepo elsewhere)"
(cd "$WORK/elsewhere-src" && git commit -q --allow-empty -m two && git push -q "$WORK/elsewhere-origin" HEAD:refs/heads/main)
(cd "$c" && git pull -q origin main)
check "a tag on another commit than the checked-out one is refused" 1 "$(tag_status "$c" v1)"

c="$(mkrepo branch)"
(cd "$WORK/branch-src" && git commit -q --allow-empty -m two && git push -q "$WORK/branch-origin" HEAD:refs/heads/v7)
check "a branch of that name is not a tag" 1 "$(tag_status "$c" v7)"

# --- gather: a name in two artifacts is an error ---------------------------
gather() { rm -rf "$WORK/g-out"; bash "$CHECK_DIR" "$1" gather "$WORK/g-out" >"$WORK/out" 2>&1; echo $?; }
mkdir -p "$WORK/g1/a" "$WORK/g1/b"
: >"$WORK/g1/a/podup-linux-x86_64"; : >"$WORK/g1/b/NOTICES.html"
check "artifacts with distinct names are gathered flat" 0 "$(gather "$WORK/g1")"
check "...into the destination" 2 "$(find "$WORK/g-out" -type f | wc -l)"
: >"$WORK/g1/b/podup-linux-x86_64"
check "two artifacts carrying the same file name are refused" 1 "$(gather "$WORK/g1")"
check "...saying which name" 1 "$(grep -c "named 'podup-linux-x86_64'" "$WORK/out")"
mkdir -p "$WORK/g2/a/deep"; : >"$WORK/g2/a/deep/x"
check "a path below the top of an artifact is refused" 1 "$(gather "$WORK/g2")"
mkdir -p "$WORK/g3/a"; : >"$WORK/g3/loose"
check "a loose file beside the artifact directories is refused" 1 "$(gather "$WORK/g3")"
mkdir -p "$WORK/g4/a"; ln -s /etc/passwd "$WORK/g4/a/podup-linux-x86_64"
check "a link inside an artifact is refused" 1 "$(gather "$WORK/g4")"

# attest mode: binaries and packages, nothing else
mkdir -p "$WORK/at1"; : >"$WORK/at1/podup-linux-x86_64"; : >"$WORK/at1/podup_5.16.0_amd64.deb"
check "binaries and packages pass the attest check" 0 "$(bash "$CHECK_DIR" "$WORK/at1" attest >/dev/null 2>&1; echo $?)"
: >"$WORK/at1/podup-5.16.0-1.glyndor.any.x86_64.rpm"; : >"$WORK/at1/podup-5.16.0-1.glyndor.any.aarch64.rpm"; : >"$WORK/at1/podup-5.16.0-1-x86_64.pkg.tar.zst"
check "the rpm and the pacman packages pass the attest check" 0 "$(bash "$CHECK_DIR" "$WORK/at1" attest >/dev/null 2>&1; echo $?)"
: >"$WORK/at1/podup-5.16.0-1.glyndor.any.riscv64.rpm"
check "a package of an architecture nobody builds is refused" 1 "$(bash "$CHECK_DIR" "$WORK/at1" attest >/dev/null 2>&1; echo $?)"
rm "$WORK/at1/podup-5.16.0-1.glyndor.any.riscv64.rpm"; : >"$WORK/at1/podup-5.16.0-1-x86_64.pkg.tar.zst.sig"
check "a signature beside a package is refused" 1 "$(bash "$CHECK_DIR" "$WORK/at1" attest >/dev/null 2>&1; echo $?)"
rm "$WORK/at1/podup-5.16.0-1-x86_64.pkg.tar.zst.sig"
: >"$WORK/at1/podup-linux-x86_64.sig"
check "a signature among them is refused" 1 "$(bash "$CHECK_DIR" "$WORK/at1" attest >/dev/null 2>&1; echo $?)"
rm "$WORK/at1/podup-linux-x86_64.sig"; : >"$WORK/at1/NOTICES.html"
check "anything else is refused before attesting" 1 "$(bash "$CHECK_DIR" "$WORK/at1" attest >/dev/null 2>&1; echo $?)"

# --- verify-provenance.sh with a stand-in gh ---------------------------------
mkdir -p "$WORK/bin" "$WORK/prov"
cat >"$WORK/bin/gh" <<'GH'
#!/usr/bin/env bash
echo "$*" >>"$GH_LOG"
[ -z "${GH_FAIL_ON:-}" ] || ! printf '%s' "$*" | grep -q "$GH_FAIL_ON"
GH
chmod +x "$WORK/bin/gh"
COMMIT_SHA="$(printf 'a%.0s' $(seq 40))"
prov() { # <version> <ref> <commit>
	SOURCE_REF="$2" SOURCE_COMMIT="$3" GH_LOG="$WORK/gh.log" PATH="$WORK/bin:$PATH" bash "$VERIFY_PROV" "$WORK/prov" Glyndor/podup "$1" >/dev/null 2>&1
	echo $?
}
: >"$WORK/gh.log"
check "every binary and package is verified against the release workflow" 0 "$(prov 5.16.0 refs/tags/v5.16.0 "$COMMIT_SHA")"
check "...eleven of them, pinned to the signer workflow" 11 "$(grep -c -- '--signer-workflow Glyndor/podup/.github/workflows/release.yml' "$WORK/gh.log")"
check "...each for the tag the release is cut from" 11 "$(grep -c -- '--source-ref refs/tags/v5.16.0' "$WORK/gh.log")"
check "...and for the commit that tag was verified to point at" 11 "$(grep -c -- "--source-digest $COMMIT_SHA" "$WORK/gh.log")"
check "...the names being the ones the version gives" 1 "$(grep -c 'podup-5.16.0-1.glyndor.any.aarch64.rpm' "$WORK/gh.log")"
check "one failed verification fails the step" 1 "$(GH_FAIL_ON=darwin-arm64 prov 5.16.0 refs/tags/v5.16.0 "$COMMIT_SHA")"
check "one failed verification of a package fails the step" 1 "$(GH_FAIL_ON=pkg.tar.zst prov 5.16.0 refs/tags/v5.16.0 "$COMMIT_SHA")"
check "a ref that is not the tag of the version is refused" 1 "$(prov 5.16.0 refs/heads/main "$COMMIT_SHA")"
check "a ref of another version is refused" 1 "$(prov 5.16.0 refs/tags/v5.15.0 "$COMMIT_SHA")"
check "a commit that is not a full hash is refused" 1 "$(prov 5.16.0 refs/tags/v5.16.0 abc123)"
check "a version that is not three numbers is refused" 1 "$(prov '5.16' refs/tags/v5.16 "$COMMIT_SHA" | awk '{ print ($1 != 0) }')"
check "no ref at all is refused" 1 "$(SOURCE_COMMIT="$COMMIT_SHA" PATH="$WORK/bin:$PATH" GH_LOG="$WORK/gh.log" bash "$VERIFY_PROV" "$WORK/prov" Glyndor/podup 5.16.0 >/dev/null 2>&1; echo $?)"

echo
echo "passed: $pass  failed: $fail"
echo "DONE release-least-privilege.test.sh $pass $fail"
[ "$fail" -eq 0 ]
