#!/usr/bin/env bash
# Behaviour tests for .github/scripts/packaging/stage-payload.py: the step that
# reads the .deb the release built and stages the files the rpm and the pacman
# builders pack. It is the one place those two packages meet input from a build
# job, so it is tested with debs made to be wrong in each way it refuses.
#
# Needs python3 only; no container, no network.
set -euo pipefail
export LC_ALL=C

here="$(cd "$(dirname "$0")" && pwd)"
script="$here/../../../../.github/scripts/packaging/stage-payload.py"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
passed=0
failed=0

# make_deb OUT CASE ARCH VERSION: a .deb for podup. CASE says what is wrong.
make_deb() {
	python3 -I - "$@" <<'PY'
import io, lzma, struct, sys, tarfile

out, case, arch, version = sys.argv[1:5]
machine = {"amd64": 62, "arm64": 183}[arch]


def elf(machine, interp=False):
    # A 64-bit little-endian ELF header with one program header: PT_LOAD, or
    # PT_INTERP for the dynamic case. Enough for the checks, not to run.
    ehdr = bytearray(64)
    ehdr[0:4] = b"\x7fELF"
    ehdr[4] = 2
    ehdr[5] = 1
    ehdr[6] = 1
    struct.pack_into("<H", ehdr, 16, 3)
    struct.pack_into("<H", ehdr, 18, machine)
    struct.pack_into("<Q", ehdr, 32, 64)
    struct.pack_into("<H", ehdr, 52, 64)
    struct.pack_into("<H", ehdr, 54, 56)
    struct.pack_into("<H", ehdr, 56, 1)
    phdr = bytearray(56)
    struct.pack_into("<I", phdr, 0, 3 if interp else 1)
    return bytes(ehdr + phdr + b"podup")


files = {
    "usr/bin/podup": (elf(machine, interp=(case == "dynamic")), 0o755),
    "usr/share/bash-completion/completions/podup": (b"bash\n", 0o644),
    "usr/share/zsh/vendor-completions/_podup": (b"zsh\n", 0o644),
    "usr/share/fish/vendor_completions.d/podup.fish": (b"fish\n", 0o644),
    "usr/share/man/man1/podup.1.gz": (b"man\n", 0o644),
    "usr/share/doc/podup/changelog.gz": (b"log\n", 0o644),
    "usr/share/doc/podup/copyright": (b"copyright\n", 0o644),
}
if case == "wrong-elf":
    files["usr/bin/podup"] = (elf(183 if machine == 62 else 62), 0o755)
if case == "not-elf":
    files["usr/bin/podup"] = (b"#!/bin/sh\n", 0o755)
if case == "missing":
    del files["usr/share/man/man1/podup.1.gz"]
if case == "extra":
    files["etc/cron.d/podup"] = (b"* * * * * root id\n", 0o644)


def tar_xz(add):
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w", format=tarfile.GNU_FORMAT) as t:
        add(t)
    return lzma.compress(raw.getvalue(), format=lzma.FORMAT_XZ)


def control(t):
    pkg_version = "9.9.9" if case == "version" else version
    pkg_arch = "arm64" if case == "control-arch" and arch == "amd64" else arch
    text = f"Package: podup\nVersion: {pkg_version}\nArchitecture: {pkg_arch}\n".encode()
    info = tarfile.TarInfo("./control")
    info.size = len(text)
    t.addfile(info, io.BytesIO(text))


def data(t):
    for name, (blob, mode) in files.items():
        name_in_tar = ("/" + name) if case == "absolute" and name == "usr/bin/podup" else "./" + name
        if case == "dotdot" and name == "usr/bin/podup":
            name_in_tar = "./usr/../../etc/podup"
        info = tarfile.TarInfo(name_in_tar)
        info.size = len(blob)
        info.mode = mode
        t.addfile(info, io.BytesIO(blob))
    if case == "symlink":
        link = tarfile.TarInfo("./usr/share/doc/podup/evil")
        link.type = tarfile.SYMTYPE
        link.linkname = "/etc/shadow"
        t.addfile(link)
    if case == "hardlink":
        link = tarfile.TarInfo("./usr/share/doc/podup/evil")
        link.type = tarfile.LNKTYPE
        link.linkname = "usr/bin/podup"
        t.addfile(link)
    if case == "device":
        dev = tarfile.TarInfo("./usr/share/doc/podup/null")
        dev.type = tarfile.CHRTYPE
        t.addfile(dev)
    if case == "duplicate":
        blob = b"again\n"
        info = tarfile.TarInfo("./usr/share/doc/podup/changelog.gz")
        info.size = len(blob)
        t.addfile(info, io.BytesIO(blob))


members = [
    ("debian-binary", b"2.0\n"),
    ("control.tar.xz", tar_xz(control)),
    ("data.tar.xz", tar_xz(data)),
]
if case == "extra-member":
    members.append(("evil.tar.xz", b"x"))
if case == "no-xz":
    members[2] = ("data.tar.gz", members[2][1])
with open(out, "wb") as f:
    f.write(b"!<arch>\n")
    for name, blob in members:
        f.write(f"{name + '/':<16}{0:<12}{0:<6}{0:<6}{'100644':<8}{len(blob):<10}`\n".encode())
        f.write(blob)
        if len(blob) % 2:
            f.write(b"\n")
PY
}

printf 'MIT License\n' >"$tmp/LICENSE"

# stage NAME DEB VERSION ARCH [extra args...] -> runs the script into $tmp/NAME
stage() {
	local name="$1" deb="$2" version="$3" arch="$4"
	shift 4
	python3 -I "$script" --deb "$deb" --version "$version" --arch "$arch" \
		--license "$tmp/LICENSE" --out "$tmp/$name" --source-date-epoch 1795380000 "$@"
}

# refuses NAME CASE ARCH VERSION EXPECTED-TEXT [stage-version]
refuses() {
	local name="$1" case_="$2" arch="$3" version="$4" want="$5" stage_version="${6:-$4}"
	make_deb "$tmp/$name.deb" "$case_" "$arch" "$version"
	local msg
	if msg="$(stage "$name" "$tmp/$name.deb" "$stage_version" "$arch" 2>&1)"; then
		echo "FAIL  $name: accepted a deb that should be refused"
		failed=$((failed + 1))
	elif [[ "$msg" == *"$want"* ]] && [ ! -e "$tmp/$name" ]; then
		echo "ok    $name"
		passed=$((passed + 1))
	else
		echo "FAIL  $name: refused with '$msg' (wanted '$want'; out dir present: $([ -e "$tmp/$name" ] && echo yes || echo no))"
		failed=$((failed + 1))
	fi
}

ok_case() { # description condition-exit-code
	if [ "$2" = 0 ]; then
		echo "ok    $1"
		passed=$((passed + 1))
	else
		echo "FAIL  $1"
		failed=$((failed + 1))
	fi
}

# --- the good deb ---------------------------------------------------------
make_deb "$tmp/good.deb" good amd64 6.0.0
stage good "$tmp/good.deb" 6.0.0 amd64 >/dev/null
ok_case "a good deb stages" "$([ -f "$tmp/good/root/usr/bin/podup" ] && echo 0 || echo 1)"
ok_case "the zsh completion moves to site-functions" \
	"$([ -f "$tmp/good/root/usr/share/zsh/site-functions/_podup" ] && [ ! -e "$tmp/good/root/usr/share/zsh/vendor-completions" ] && echo 0 || echo 1)"
ok_case "the Debian copyright file is not staged, the repository LICENSE is" \
	"$([ ! -e "$tmp/good/root/usr/share/doc/podup/copyright" ] && [ -f "$tmp/good/root/usr/share/licenses/podup/LICENSE" ] && echo 0 || echo 1)"
ok_case "the binary is 0755 and the rest 0644" \
	"$([ "$(stat -c %a "$tmp/good/root/usr/bin/podup")" = 755 ] && [ "$(stat -c %a "$tmp/good/root/usr/share/man/man1/podup.1.gz")" = 644 ] && echo 0 || echo 1)"
ok_case "every staged mtime is SOURCE_DATE_EPOCH" \
	"$([ "$(find "$tmp/good/root" -printf '%T@\n' | sort -u | cut -d. -f1)" = 1795380000 ] && echo 0 || echo 1)"

# The same deb staged twice lists the same files.
stage good2 "$tmp/good.deb" 6.0.0 amd64 >/dev/null
ok_case "staging twice gives the same listing" "$(cmp -s "$tmp/good/files.txt" "$tmp/good2/files.txt" && echo 0 || echo 1)"

make_deb "$tmp/arm.deb" good arm64 6.0.0
stage arm "$tmp/arm.deb" 6.0.0 arm64 >/dev/null
ok_case "an arm64 deb stages as arm64" "$([ -f "$tmp/arm/root/usr/bin/podup" ] && echo 0 || echo 1)"

# --- refusals -------------------------------------------------------------
refuses version-differs version amd64 6.0.0 "control says Version"
refuses stage-version-differs good amd64 6.0.0 "control says Version" 6.0.1
refuses arch-in-control control-arch amd64 6.0.0 "control says Architecture"
refuses arch-of-binary wrong-elf amd64 6.0.0 "another architecture"
refuses dynamic dynamic amd64 6.0.0 "dynamic loader"
refuses not-elf not-elf amd64 6.0.0 "not an ELF"
refuses symlink symlink amd64 6.0.0 "not a regular file"
refuses hardlink hardlink amd64 6.0.0 "not a regular file"
refuses device device amd64 6.0.0 "not a regular file"
refuses absolute absolute amd64 6.0.0 "unsafe path"
refuses dotdot dotdot amd64 6.0.0 "unsafe path"
refuses extra-file extra amd64 6.0.0 "does not know"
refuses missing-file missing amd64 6.0.0 "lacks"
refuses duplicate duplicate amd64 6.0.0 "twice"
refuses extra-member extra-member amd64 6.0.0 "unexpected .deb members"
refuses other-compression no-xz amd64 6.0.0 "unexpected .deb members"

# The binary built for the other architecture, under a control that agrees.
python3 -I - "$tmp" <<'PY'
import sys
tmp = sys.argv[1]
data = open(f"{tmp}/arm.deb", "rb").read()
open(f"{tmp}/swapped.deb", "wb").write(data)
PY
if msg="$(stage swapped "$tmp/swapped.deb" 6.0.0 amd64 2>&1)"; then
	echo "FAIL  an arm64 deb was staged as amd64"
	failed=$((failed + 1))
else
	if [[ "$msg" == *"control says Architecture"* ]]; then
		echo "ok    an arm64 deb is not staged as amd64"
		passed=$((passed + 1))
	else
		echo "FAIL  swapped: $msg"
		failed=$((failed + 1))
	fi
fi

# --- arguments ------------------------------------------------------------
bad_arg() { # name expected-text args...
	local name="$1" want="$2"
	shift 2
	local msg
	if msg="$(python3 -I "$script" "$@" 2>&1)"; then
		echo "FAIL  $name: accepted"
		failed=$((failed + 1))
	elif [[ "$msg" == *"$want"* ]]; then
		echo "ok    $name"
		passed=$((passed + 1))
	else
		echo "FAIL  $name: '$msg'"
		failed=$((failed + 1))
	fi
}
for v in "v6.0.0" "6.0" "6.0.0-1" "6.0.0 " "6.0.0;id" "\$(id)" "../6.0.0" "6.0.0+r1"; do
	bad_arg "version '$v'" "is not X.Y.Z" --deb "$tmp/good.deb" --version "$v" --arch amd64 --license "$tmp/LICENSE" --out "$tmp/v" --source-date-epoch 1
done
for e in "" "abc" "-1" "1.5" "1e9" "9999999999999"; do
	bad_arg "epoch '$e'" "SOURCE_DATE_EPOCH" --deb "$tmp/good.deb" --version 6.0.0 --arch amd64 --license "$tmp/LICENSE" --out "$tmp/e" --source-date-epoch "$e"
done
bad_arg "an existing output directory" "exists" --deb "$tmp/good.deb" --version 6.0.0 --arch amd64 --license "$tmp/LICENSE" --out "$tmp/good" --source-date-epoch 1
ln -s "$tmp/good.deb" "$tmp/link.deb"
bad_arg "a deb that is a link" "not a regular file" --deb "$tmp/link.deb" --version 6.0.0 --arch amd64 --license "$tmp/LICENSE" --out "$tmp/l" --source-date-epoch 1
bad_arg "an architecture that is not amd64 or arm64" "invalid choice" --deb "$tmp/good.deb" --version 6.0.0 --arch riscv64 --license "$tmp/LICENSE" --out "$tmp/r" --source-date-epoch 1
: >"$tmp/empty"
bad_arg "an empty licence" "LICENSE is empty" --deb "$tmp/good.deb" --version 6.0.0 --arch amd64 --license "$tmp/empty" --out "$tmp/emp" --source-date-epoch 1

printf 'passed %d, failed %d\n' "$passed" "$failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$passed" "$failed"
if [ "$failed" -ne 0 ]; then exit 1; fi
