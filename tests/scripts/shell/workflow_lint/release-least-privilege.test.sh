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

# --- the shipped workflow, read as data ------------------------------------
facts="$(python3 - "$RELEASE" <<'PY'
import sys, yaml
jobs = yaml.safe_load(open(sys.argv[1]))["jobs"]

def text(job):
    return yaml.safe_dump(job)

out = {}
refs = [s["with"]["ref"] for j in jobs.values() for s in j["steps"]
        if str(s.get("uses", "")).startswith("actions/checkout")]
out["checkout refs all name the tag"] = all(r.startswith("refs/tags/") for r in refs)
out["verify checkout keeps no credential"] = next(
    s for s in jobs["verify"]["steps"] if str(s.get("uses", "")).startswith("actions/checkout")
)["with"].get("persist-credentials") is False
names = [s.get("name", "") for s in jobs["verify"]["steps"]]
tag = next(i for i, s in enumerate(jobs["verify"]["steps"]) if "check-tag.sh" in str(s.get("run", "")))
install = next(i for i, s in enumerate(jobs["verify"]["steps"]) if "cargo install" in str(s.get("run", "")))
out["the tag is checked before anything is installed"] = tag < install
out["build holds no minting permission"] = not (set(jobs["build"]["permissions"]) & {"id-token", "attestations"})
out["build-deb holds no minting permission"] = not (set(jobs["build-deb"]["permissions"]) & {"id-token", "attestations"})
out["no attestation step in build or build-deb"] = not any(
    "attest-build-provenance" in text(jobs[j]) for j in ("build", "build-deb"))
a = jobs["attest"]
out["attest compiles nothing"] = "cargo" not in text(a)
out["attest waits for both builds"] = set(a["needs"]) == {"build", "build-deb"}
out["attest checks the names before attesting"] = [
    i for i, s in enumerate(a["steps"]) if "check-artifact-dir.sh" in str(s.get("run", ""))
][0] < [i for i, s in enumerate(a["steps"]) if "attest-build-provenance" in str(s.get("uses", ""))][0]
rel = jobs["release"]
out["release waits for attest"] = "attest" in rel["needs"]
dl = next(s for s in rel["steps"] if "download-artifact" in str(s.get("uses", "")))
out["release does not merge on download"] = "merge-multiple" not in dl.get("with", {})
steps = rel["steps"]
def at(needle):
    return next(i for i, s in enumerate(steps) if needle in str(s.get("run", "")))
out["release gathers with duplicate detection"] = "gather" in str(steps[at("check-artifact-dir.sh")]["run"])
out["provenance is verified before signing"] = at("verify-provenance.sh") < next(
    i for i, s in enumerate(steps) if "ED25519_KEY" in str(s))
out["the provenance step cannot be skipped"] = "if" not in steps[at("verify-provenance.sh")] and not steps[at("verify-provenance.sh")].get("continue-on-error")
out["release may read attestations"] = rel["permissions"].get("attestations") == "read"
out["the sign job's signature artifacts do not match podup-*"] = all(
    not s["with"]["name"].startswith("podup-")
    for s in jobs["sign"]["steps"] if "upload-artifact" in str(s.get("uses", "")))
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
mkdir -p "$WORK/at1"; : >"$WORK/at1/podup-linux-x86_64"; : >"$WORK/at1/podup_5.16.0_amd64.deb"; 
check "binaries and packages pass the attest check" 0 "$(bash "$CHECK_DIR" "$WORK/at1" attest >/dev/null 2>&1; echo $?)"
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
for n in podup-linux-x86_64 podup-linux-arm64 podup-darwin-arm64 podup-darwin-x86_64 podup-windows-x86_64.exe podup-windows-arm64.exe podup_5.16.0_amd64.deb podup_5.16.0_arm64.deb; do : >"$WORK/prov/$n"; done
prov() { GH_LOG="$WORK/gh.log" PATH="$WORK/bin:$PATH" bash "$VERIFY_PROV" "$WORK/prov" Glyndor/podup >/dev/null 2>&1; echo $?; }
: >"$WORK/gh.log"
check "every binary and package is verified against the release workflow" 0 "$(prov)"
check "...eight of them, pinned to the signer workflow" 8 "$(grep -c -- '--signer-workflow Glyndor/podup/.github/workflows/release.yml' "$WORK/gh.log")"
check "one failed verification fails the step" 1 "$(GH_FAIL_ON=darwin-arm64 prov)"

echo
echo "passed: $pass  failed: $fail"
echo "DONE release-least-privilege.test.sh $pass $fail"
[ "$fail" -eq 0 ]
