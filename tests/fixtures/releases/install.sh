#!/usr/bin/env bash
# Fixture install.sh for the release-signing regression test.
# Two rotation slots, the number the consumers read. Slot 1 holds a keypair
# that does NOT match the test signatures; slot 2 holds the test key that
# signs every other asset in this directory.

PODUP_RELEASE_PUBKEY_B64="${PODUP_RELEASE_PUBKEY_B64:-iojj3XQJ8ZX9UtstPLpdcspnCb8dlBIb83SIAbQPb1w}"
PODUP_RELEASE_PUBKEY2_B64="${PODUP_RELEASE_PUBKEY2_B64:-6kpsY+KcUgq+9VB7Ey7F+ZVHdq6+vnuSQh7qaRRG0iw}"

PUBKEYS=()
[[ -n "$PODUP_RELEASE_PUBKEY_B64"  ]] && PUBKEYS+=("$PODUP_RELEASE_PUBKEY_B64")
[[ -n "$PODUP_RELEASE_PUBKEY2_B64" ]] && PUBKEYS+=("$PODUP_RELEASE_PUBKEY2_B64")
