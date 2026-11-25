#!/usr/bin/env python3
"""The exact names a release is made of, derived from the version the tag was verified to carry.

Usage: python3 -I expected-assets.py <version> <set>

<set> is one of:
  binaries     the six binaries
  packages     the packages: two .deb, two .rpm and one pacman package
  sboms        one SBOM per binary and per .deb
  checksummed  what SHA256SUMS lists: the binaries, the packages, the SBOMs, NOTICES.html and the installers
  staged       what the sealing job starts from: the checksummed files, without SHA256SUMS, and the
               signature of every binary and package (the jobs that sign those have already run)
  sealed       what the sealing job signs itself: SHA256SUMS, the SBOMs, NOTICES.html and the installers
  published    everything a release carries: the checksummed files and SHA256SUMS, each with its .sig

One name per line, sorted with the C collation. Everything that decides what a release is made of
(a pattern in a script, a glob in a workflow, a signing loop) is held against this list, so a file that
is missing, one that is extra and a name nobody asked for are each a difference, not a pass. The
version is three numbers, so a name cannot be shaped by what the caller passes.
"""
import re
import sys

BINARIES = [
    "podup-linux-x86_64",
    "podup-linux-arm64",
    "podup-darwin-arm64",
    "podup-darwin-x86_64",
    "podup-windows-x86_64.exe",
    "podup-windows-arm64.exe",
]
PLAIN = ["NOTICES.html", "install.sh", "install.ps1"]


def names(version):
    debs = [f"podup_{version}_amd64.deb", f"podup_{version}_arm64.deb"]
    packages = debs + [
        f"podup-{version}-1.glyndor.any.x86_64.rpm",
        f"podup-{version}-1.glyndor.any.aarch64.rpm",
        f"podup-{version}-1-x86_64.pkg.tar.zst",
    ]
    sboms = [f"{n}.cdx.json" for n in BINARIES + debs]
    checksummed = BINARIES + packages + sboms + PLAIN
    sigs = lambda files: [f"{f}.sig" for f in files]
    return {
        "binaries": BINARIES,
        "packages": packages,
        "sboms": sboms,
        "checksummed": checksummed,
        "staged": checksummed + sigs(BINARIES + packages),
        "sealed": ["SHA256SUMS"] + sboms + PLAIN,
        "published": checksummed + ["SHA256SUMS"] + sigs(checksummed + ["SHA256SUMS"]),
    }


def main(argv):
    if len(argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    version, which = argv[1], argv[2]
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
        print(f"expected-assets: '{version}' is not three numbers", file=sys.stderr)
        return 2
    sets = names(version)
    if which not in sets:
        print(f"expected-assets: unknown set '{which}'; one of {', '.join(sets)}", file=sys.stderr)
        return 2
    wanted = sets[which]
    if len(set(wanted)) != len(wanted):
        print("expected-assets: a name is listed twice", file=sys.stderr)
        return 2
    print("\n".join(sorted(wanted)))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
