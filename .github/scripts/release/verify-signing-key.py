#!/usr/bin/env python3
"""Prove a freshly signed artifact verifies against the keys consumers embed.

Usage:
  verify-signing-key.py <signed-file> [<sig-file>] [<installer-path>]

sign.py checks only that GLYNDOR_RELEASE_ED25519_KEY is base64 that decodes to
32 bytes. A well-formed but WRONG seed therefore signs happily, and the release
publishes with a signature no installer, updater or apt client can verify - a
dead release, and releases are immutable, so the version is burnt and the fix is
a whole new one. The signing secret and the embedded keys are edited in different
places at different times, and nothing compared them.

This closes the loop by verifying the produced signature against the public keys
that ship to users, read straight out of install.sh rather than restated here -
a copy in this file could drift from the installer and would prove nothing about
what a user actually runs. Any populated slot may verify (rotation trusts two
keys at once); the check fails only when NO embedded key accepts the signature,
which is exactly the condition that would strand every consumer.

The number of slots is SLOT_COUNT, and it is the number the consumers verify
against, not a wider one: a key in a slot that no installer, updater or channel
reads would make the release pass here and be refused everywhere else. The
installer is refused when it declares more slots than that, and
check-key-slots.py holds the same number against install.sh, install.ps1 and
internal/update/verify.rs. Taking a third slot into use is a change to that one
number and to the three consumers, made together.

The optional third argument points the verifier at an installer other than the
one at the repo root - the test fixture ships a two-slot install.sh.

PODUP_VERIFY_ONLY_KEY_B64, when set, replaces the embedded keys with that one
key. A rehearsal of the release signs with a throwaway key that no consumer
embeds, and verifies against it; a real release never sets it.

Exit: 0 verified, 1 mismatch or malformed input.
"""
import base64
import os
import re
import sys
from pathlib import Path

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

DEFAULT_INSTALLER = Path(__file__).resolve().parents[3] / "install.sh"

# The number of key slots the consumers verify against. install.sh, install.ps1
# and internal/update/verify.rs each have exactly this many, and
# check-key-slots.py compares them with it.
SLOT_COUNT = 2

# The slots are `PODUP_RELEASE_PUBKEY_B64` (the first) and `PODUP_RELEASE_PUBKEY<n>_B64`
# for n from 2 to SLOT_COUNT. Any other slot name, a third one included, is not a slot
# the consumers read, and is refused below. The default is taken from the
# `${VAR:-DEFAULT}` form so CI reads the same literal a user gets when the env
# override is unset.
DECLARED_RE = re.compile(
	r'^PODUP_RELEASE_PUBKEY([0-9]*)_B64="\$\{PODUP_RELEASE_PUBKEY([0-9]*)_B64:-([^}]*)\}"$',
	re.MULTILINE,
)
SLOT_NUMBERS = [""] + [str(n) for n in range(2, SLOT_COUNT + 1)]


def embedded_pubkeys(installer: Path) -> list[str]:
	"""Return the non-empty release pubkeys the installer ships, in slot order."""
	if not installer.is_file():
		sys.exit(f"cannot read {installer}; refusing to publish unverified")
	declared = DECLARED_RE.findall(installer.read_text(encoding="utf-8"))
	if not declared:
		sys.exit(f"found no PODUP_RELEASE_PUBKEY*_B64 slots in {installer}")
	for name, again, _value in declared:
		if name != again:
			sys.exit(f"{installer}: slot {name or 1} defaults from the variable of slot {again or 1}")
	numbers = [name for name, _again, _value in declared]
	if numbers != SLOT_NUMBERS:
		sys.exit(
			f"{installer} declares the slots {[n or '1' for n in numbers]}, and the consumers read "
			f"{[n or '1' for n in SLOT_NUMBERS]}: a key in a slot nobody reads would pass here and be "
			"refused everywhere else. Change SLOT_COUNT, install.sh, install.ps1 and "
			"internal/update/verify.rs together."
		)
	return [value for _name, _again, value in declared if value]


def main() -> None:
	if len(sys.argv) < 2:
		print(__doc__, file=sys.stderr)
		sys.exit(1)

	signed = Path(sys.argv[1])
	sig = Path(sys.argv[2]) if len(sys.argv) > 2 else Path(str(signed) + ".sig")
	installer = Path(sys.argv[3]) if len(sys.argv) > 3 else DEFAULT_INSTALLER

	data = signed.read_bytes()
	signature = sig.read_bytes()

	only = os.environ.get("PODUP_VERIFY_ONLY_KEY_B64", "")
	keys = [only] if only else embedded_pubkeys(installer)
	if not keys:
		sys.exit("every embedded key slot is empty; nothing could verify this release")

	for slot, key_b64 in enumerate(keys):
		pad = "=" * (-len(key_b64) % 4)
		try:
			raw = base64.b64decode(key_b64 + pad, validate=True)
			Ed25519PublicKey.from_public_bytes(raw).verify(signature, data)
		except (ValueError, base64.binascii.Error) as exc:
			sys.exit(f"embedded key slot {slot} is not a valid Ed25519 public key: {exc}")
		except InvalidSignature:
			continue
		print(f"{signed.name}: verified against embedded key slot {slot} ({key_b64})")
		return

	sys.exit(
		f"{signed.name}: the signature GLYNDOR_RELEASE_ED25519_KEY just produced "
		f"verifies against NONE of the {len(keys)} embedded key(s): {', '.join(keys)}.\n"
		"The signing secret and the keys baked into install.sh / install.ps1 / "
		"internal/update/verify.rs disagree, so this release would be unverifiable "
		"and uninstallable. Refusing to publish. Fix whichever side is wrong before "
		"retagging; the tag is immutable once the release exists."
	)


if __name__ == "__main__":
	main()
