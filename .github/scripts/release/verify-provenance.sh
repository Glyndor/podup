#!/usr/bin/env bash
#
# Usage: verify-provenance.sh DIR REPO
#
# Run in the release job, before anything is signed: every binary and package in
# DIR must carry a build-provenance attestation made by this repository's release
# workflow, so what gets signed is the bytes the attestation covers.
set -euo pipefail

dir="${1:?directory}"
repo="${2:?owner/repo}"

cd "$dir"
for file in podup-linux-x86_64 podup-linux-arm64 podup-darwin-arm64 podup-darwin-x86_64 \
	podup-windows-x86_64.exe podup-windows-arm64.exe podup_*_amd64.deb podup_*_arm64.deb; do
	gh attestation verify "$file" --repo "$repo" \
		--signer-workflow "$repo/.github/workflows/release.yml"
done
