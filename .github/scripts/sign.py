#!/usr/bin/env python3
"""Sign a file with an Ed25519 private key (raw 32-byte seed, base64-encoded).

Usage:
  sign.py <base64-private-key> <input-file> [<output-sig-file>]

If output-sig-file is omitted, writes to <input-file>.sig.
The key must be the raw 32-byte Ed25519 seed in standard base64.
"""
import base64
import sys

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey


def main() -> None:
	if len(sys.argv) < 3:
		print(__doc__, file=sys.stderr)
		sys.exit(1)

	key_b64 = sys.argv[1]
	input_file = sys.argv[2]
	sig_file = sys.argv[3] if len(sys.argv) > 3 else input_file + ".sig"

	key_bytes = base64.b64decode(key_b64 + "==")
	private_key = Ed25519PrivateKey.from_private_bytes(key_bytes)

	with open(input_file, "rb") as f:
		data = f.read()

	sig = private_key.sign(data)

	with open(sig_file, "wb") as f:
		f.write(sig)

	print(f"signed {input_file} ({len(data):,} bytes) → {sig_file} ({len(sig)} bytes)")


if __name__ == "__main__":
	main()
