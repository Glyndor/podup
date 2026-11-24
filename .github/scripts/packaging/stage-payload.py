#!/usr/bin/env python3
"""Stage the files of the podup .deb for the rpm and pacman builders.

The release builds the .deb once, from the pinned musl toolchain, with the
features a package-managed install ships (no self-update: the package manager
owns upgrades). The rpm and the pacman package are made from that payload, so
the three install the same bytes. This script reads the .deb, checks it is the
package it should be, and writes the tree the builders pack.

    python3 -I stage-payload.py --deb FILE --version X.Y.Z --arch amd64|arm64 \
        --license FILE --out DIR [--source-date-epoch N]

DIR receives `root/` (the tree) and `files.txt` (one line per file:
`<octal mode> <path> <sha256> <size>`, sorted by path). Nothing is executed
from the .deb. Standard library only; run it with `-I`.

It refuses, before writing anything, a .deb that is not exactly the expected
shape: other members, links, absolute or `..` paths, a binary of another
architecture or one linked against a shared C library, a version that is not
the one given.
"""

import argparse
import hashlib
import io
import lzma
import os
import re
import shutil
import sys
import tarfile

VERSION_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")
ARCHES = {"amd64": ("x86_64", 62), "arm64": ("aarch64", 183)}

MAX_DEB = 300 * 1024 * 1024
MAX_MEMBER = 200 * 1024 * 1024

# The data member's files and where each one lands. The zsh completion is the
# one file that does not keep its Debian path: `vendor-completions` is a Debian
# patch to zsh, and neither Fedora, openSUSE nor Arch put it on `fpath`.
# `site-functions` is where all of them look.
PAYLOAD = {
    "usr/bin/podup": ("usr/bin/podup", 0o755),
    "usr/share/bash-completion/completions/podup": (
        "usr/share/bash-completion/completions/podup",
        0o644,
    ),
    "usr/share/zsh/vendor-completions/_podup": (
        "usr/share/zsh/site-functions/_podup",
        0o644,
    ),
    "usr/share/fish/vendor_completions.d/podup.fish": (
        "usr/share/fish/vendor_completions.d/podup.fish",
        0o644,
    ),
    "usr/share/man/man1/podup.1.gz": ("usr/share/man/man1/podup.1.gz", 0o644),
    "usr/share/doc/podup/changelog.gz": ("usr/share/doc/podup/changelog.gz", 0o644),
}
# Debian's own licence file (Debian copyright format) is not shipped to rpm or
# pacman; the repository's LICENSE goes where both look for it.
IGNORED = {"usr/share/doc/podup/copyright"}
LICENSE_DEST = ("usr/share/licenses/podup/LICENSE", 0o644)


def die(message):
    print(f"stage-payload: {message}", file=sys.stderr)
    sys.exit(1)


def read_ar(data):
    """The members of an ar archive as {name: bytes}, in order, no duplicates."""
    if data[:8] != b"!<arch>\n":
        die("not an ar archive")
    members, pos = [], 8
    while pos < len(data):
        header = data[pos : pos + 60]
        if len(header) < 60 or header[58:60] != b"`\n":
            die("damaged ar header")
        name = header[0:16].decode("ascii", "replace").strip().rstrip("/")
        try:
            size = int(header[48:58].decode("ascii").strip())
        except ValueError:
            die("damaged ar size")
        pos += 60
        if size < 0 or pos + size > len(data):
            die("ar member runs past the end")
        members.append((name, data[pos : pos + size]))
        pos += size + (size & 1)
    names = [n for n, _ in members]
    if len(set(names)) != len(names):
        die("an ar member name appears twice")
    return members


def read_tar_xz(blob, what):
    try:
        raw = lzma.LZMADecompressor().decompress(blob, max_length=MAX_MEMBER + 1)
    except lzma.LZMAError as e:
        die(f"{what} is not xz: {e}")
    if len(raw) > MAX_MEMBER:
        die(f"{what} is larger than {MAX_MEMBER} bytes once decompressed")
    return tarfile.open(fileobj=io.BytesIO(raw), mode="r:")


def control_fields(tar):
    members = [m for m in tar.getmembers() if normalise(m.name) == "control"]
    if len(members) != 1 or not members[0].isreg() or members[0].size > 65536:
        die("control.tar.xz has no single regular `control` file")
    text = tar.extractfile(members[0]).read().decode("utf-8", "replace")
    fields = {}
    for line in text.splitlines():
        if line and not line[0].isspace() and ":" in line:
            key, value = line.split(":", 1)
            fields.setdefault(key, value.strip())
    return fields


def check_elf(blob, machine):
    if len(blob) < 64 or blob[:4] != b"\x7fELF":
        die("usr/bin/podup is not an ELF file")
    if blob[4] != 2 or blob[5] != 1:
        die("usr/bin/podup is not a 64-bit little-endian ELF")
    if int.from_bytes(blob[18:20], "little") != machine:
        die("usr/bin/podup is built for another architecture than the package")
    phoff = int.from_bytes(blob[32:40], "little")
    phentsize = int.from_bytes(blob[54:56], "little")
    phnum = int.from_bytes(blob[56:58], "little")
    if phentsize < 56 or phoff + phentsize * phnum > len(blob):
        die("usr/bin/podup has a damaged program header table")
    for i in range(phnum):
        p_type = int.from_bytes(blob[phoff + i * phentsize :][:4], "little")
        if p_type == 3:  # PT_INTERP: a dynamically linked binary
            die("usr/bin/podup asks for a dynamic loader; the package must carry a static binary")


def normalise(name):
    return name.removeprefix("./")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--deb", required=True)
    ap.add_argument("--version", required=True)
    ap.add_argument("--arch", required=True, choices=sorted(ARCHES))
    ap.add_argument("--license", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument(
        "--source-date-epoch",
        default=os.environ.get("SOURCE_DATE_EPOCH", ""),
    )
    args = ap.parse_args()

    if not VERSION_RE.match(args.version):
        die(f"version {args.version!r} is not X.Y.Z")
    if not re.fullmatch(r"[0-9]{1,12}", args.source_date_epoch):
        die("SOURCE_DATE_EPOCH (or --source-date-epoch) must be a number of seconds")
    epoch = int(args.source_date_epoch)
    rpm_arch, machine = ARCHES[args.arch]

    for path in (args.deb, args.license):
        if os.path.islink(path) or not os.path.isfile(path):
            die(f"{path} is not a regular file")
    if os.path.getsize(args.deb) > MAX_DEB:
        die("the .deb is larger than 300 MiB")
    with open(args.deb, "rb") as f:
        members = read_ar(f.read())
    names = [n for n, _ in members]
    if names != ["debian-binary", "control.tar.xz", "data.tar.xz"]:
        die(f"unexpected .deb members: {names}")
    blobs = dict(members)
    if blobs["debian-binary"] != b"2.0\n":
        die("not a format 2.0 .deb")

    fields = control_fields(read_tar_xz(blobs["control.tar.xz"], "control.tar.xz"))
    expect = {"Package": "podup", "Version": args.version, "Architecture": args.arch}
    for key, want in expect.items():
        if fields.get(key) != want:
            die(f"control says {key}: {fields.get(key)!r}, expected {want!r}")

    data = read_tar_xz(blobs["data.tar.xz"], "data.tar.xz")
    found = {}
    for m in data.getmembers():
        name = normalise(m.name).rstrip("/")
        if name in ("", "."):
            continue
        if name.startswith("/") or ".." in name.split("/"):
            die(f"unsafe path in the .deb: {m.name!r}")
        if m.isdir():
            continue
        if not m.isreg():
            die(f"{m.name!r} is not a regular file or directory")
        if name in IGNORED:
            continue
        if name not in PAYLOAD:
            die(f"the .deb carries a file this packaging does not know: {name}")
        if name in found:
            die(f"{name} appears twice")
        if m.size > MAX_MEMBER:
            die(f"{name} is too large")
        found[name] = data.extractfile(m).read()
    missing = sorted(set(PAYLOAD) - set(found))
    if missing:
        die(f"the .deb lacks: {', '.join(missing)}")
    check_elf(found["usr/bin/podup"], machine)

    with open(args.license, "rb") as f:
        licence = f.read(65537)
    if not licence or len(licence) > 65536:
        die("LICENSE is empty or larger than 64 KiB")

    out = args.out
    if os.path.lexists(out):
        die(f"{out} exists; give a new directory")
    root = os.path.join(out, "root")
    os.makedirs(root)

    entries = []
    for src, (dest, mode) in PAYLOAD.items():
        entries.append((dest, mode, found[src]))
    entries.append((LICENSE_DEST[0], LICENSE_DEST[1], licence))
    entries.sort()

    dirs = set()
    for dest, mode, blob in entries:
        target = os.path.join(root, dest)
        os.makedirs(os.path.dirname(target), exist_ok=True)
        with open(target, "wb") as f:
            f.write(blob)
        os.chmod(target, mode)
        parent = os.path.dirname(dest)
        while parent:
            dirs.add(parent)
            parent = os.path.dirname(parent)
    # Every mtime is the release's, files and directories alike, and every
    # directory has the same mode, so the tree does not depend on the runner.
    for d in sorted(dirs):
        os.chmod(os.path.join(root, d), 0o755)
    for dest, _, _ in entries:
        os.utime(os.path.join(root, dest), (epoch, epoch))
    for d in sorted(dirs, key=len, reverse=True):
        os.utime(os.path.join(root, d), (epoch, epoch))
    os.chmod(root, 0o755)
    os.utime(root, (epoch, epoch))

    with open(os.path.join(out, "files.txt"), "w", encoding="ascii") as listing:
        for dest, mode, blob in entries:
            listing.write(f"{mode:04o} {dest} {hashlib.sha256(blob).hexdigest()} {len(blob)}\n")
    print(f"staged {len(entries)} files for podup {args.version} ({rpm_arch})")


if __name__ == "__main__":
    main()
