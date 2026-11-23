#!/usr/bin/env python3
"""The public key of the seed in REHEARSAL_SEED_B64, unpadded base64, on stdout.

Run with -I. Used by a rehearsal of the release, which signs with a throwaway seed that no
consumer embeds and verifies against the key that seed makes. It reads no secret.
"""
import base64
import os
import sys

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

seed_b64 = os.environ.get("REHEARSAL_SEED_B64", "").strip()
try:
	seed = base64.b64decode(seed_b64 + "=" * (-len(seed_b64) % 4), validate=True)
except ValueError:
	sys.exit("REHEARSAL_SEED_B64 is not base64")
if len(seed) != 32:
	sys.exit(f"REHEARSAL_SEED_B64 must decode to 32 bytes, got {len(seed)}")
raw = Ed25519PrivateKey.from_private_bytes(seed).public_key().public_bytes(
	serialization.Encoding.Raw, serialization.PublicFormat.Raw
)
print(base64.b64encode(raw).decode().rstrip("="))
