#!/usr/bin/env bash
#
# Usage: verify-signatures.sh VENV_PYTHON VERSION
# Run in the folder that holds the files of the release. VENV_PYTHON is the interpreter of a
# virtualenv with the pinned `cryptography`; VERSION is the version being released.
#
# sign.py only checks the seed is base64 of 32 bytes, so a wrong-but-well-formed
# GLYNDOR_RELEASE_ED25519_KEY signs happily and publishes a release nothing can verify.
# Releases are immutable, so that mistake burns the version. Every asset is verified, each
# with its own .sig, because a per-binary substitution is the failure SHA256SUMS cannot
# catch (the manifest can be re-signed to match the swapped binary). The list of what
# carries a signature is the one the version gives, so an empty directory of signatures
# cannot pass: a missing signature and an extra one are both a difference from the exact set.
#
# In a rehearsal (REHEARSAL=true) the files were signed with the public throwaway seed in
# REHEARSAL_SEED_B64, and they are verified against the key that seed makes and against no
# key a consumer embeds.
set -euo pipefail
export LC_ALL=C

[ "$#" -eq 2 ] || {
	echo "usage: verify-signatures.sh VENV_PYTHON VERSION" >&2
	exit 2
}
venv_py="$1"
version="$2"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [ "${REHEARSAL:-}" = true ]; then
	PODUP_VERIFY_ONLY_KEY_B64="$("$venv_py" -I "$here/public-key-of.py")"
	export PODUP_VERIFY_ONLY_KEY_B64
fi
count=0
want=$(($(python3 -I "$here/expected-assets.py" "$version" checksummed | wc -l) + 1))
while IFS= read -r file; do
	"$venv_py" -I "$here/verify-signing-key.py" "$file" "$file.sig"
	count=$((count + 1))
done < <({
	python3 -I "$here/expected-assets.py" "$version" checksummed
	echo SHA256SUMS
})
[ "$count" -eq "$want" ] || {
	echo "::error::$count signatures were verified, $want were expected" >&2
	exit 1
}
