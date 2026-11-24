#!/usr/bin/env python3
"""The number of release key slots is the same everywhere a key is read.

Usage: python3 -I check-key-slots.py [<repository root>]

verify-signing-key.py is the gate that decides, before anything is published, whether the
signature of a release verifies against what consumers embed. It holds the number of slots
(SLOT_COUNT). A consumer that reads fewer slots than the gate accepts turns a release the
gate passed into one that nobody can install, and releases cannot be taken back. So the
same number has to be in:

  - install.sh: the slot variables declared, and the PUBKEYS+=(...) lines that put them
    in the list the installer verifies against;
  - install.ps1: the $PubKey<n>B64 variables and the ones that go in $PubKeys;
  - internal/update/verify.rs: the length of RELEASE_PUBKEYS, which is what `podup update`
    verifies against, and the same for the apt keyring, which the test of the key copies
    checks by value.

Each count is printed and compared; any difference, or a count that cannot be read, fails.
Exit 0 when they agree, 1 when they differ, 2 when something cannot be read.
"""
import re
import sys
from pathlib import Path


def read(root, rel):
    path = root / rel
    try:
        return path.read_text(encoding="utf-8")
    except OSError as e:
        print(f"check-key-slots: cannot read {rel}: {e}", file=sys.stderr)
        sys.exit(2)


def one(pattern, text, what, flags=0):
    found = re.findall(pattern, text, flags)
    if len(found) != 1:
        print(f"check-key-slots: expected exactly one {what}, found {len(found)}", file=sys.stderr)
        sys.exit(2)
    return found[0]


def counts(root):
    gate = read(root, ".github/scripts/release/verify-signing-key.py")
    sh = read(root, "install.sh")
    ps1 = read(root, "install.ps1")
    rs = read(root, "internal/update/verify.rs")
    return {
        "the gate (SLOT_COUNT in verify-signing-key.py)": int(one(r"^SLOT_COUNT = ([0-9]+)$", gate, "SLOT_COUNT", re.M)),
        "install.sh, slots declared": len(re.findall(r'^PODUP_RELEASE_PUBKEY[0-9]*_B64="\$\{', sh, re.M)),
        "install.sh, slots in PUBKEYS": len(re.findall(r'^\[\[ -n "\$PODUP_RELEASE_PUBKEY[0-9]*_B64"\s*\]\]\s*&&\s*PUBKEYS\+=', sh, re.M)),
        "install.ps1, slots declared": len(re.findall(r"^\s*\$PubKey[0-9]*B64\s*=", ps1, re.M)),
        "install.ps1, slots in $PubKeys": len(
            re.findall(r"\$PubKey[0-9]*B64", one(r"^\s*\$PubKeys\s*=\s*@\((.*?)\|", ps1, "the $PubKeys list", re.M))
        ),
        "internal/update/verify.rs, RELEASE_PUBKEYS": int(one(r"pub const RELEASE_PUBKEYS: \[\[u8; 32\]; ([0-9]+)\]", rs, "RELEASE_PUBKEYS")),
    }


def main(argv):
    root = Path(argv[1]) if len(argv) > 1 else Path(__file__).resolve().parents[3]
    found = counts(root)
    for what, n in found.items():
        print(f"{n}  {what}")
    if len(set(found.values())) != 1:
        print("check-key-slots: the number of key slots is not the same everywhere; a release signed with a slot "
              "that a consumer does not read would pass the gate and be refused by that consumer", file=sys.stderr)
        return 1
    print(f"check-key-slots: {next(iter(found.values()))} slots, everywhere.")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
