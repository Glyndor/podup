#!/usr/bin/env bash
# Fail when a tracked file points at a document tree this repository does not
# have.
#
# The Rust test `no_private_document_references` reads every tracked file for
# the same two needles. It sits behind the Rust lanes, and the classifier
# (`rust-ci-inputs.sh`) lets a pull request that touches only a shell test,
# the licence or an issue template skip those lanes, so the Rust test does
# not run for exactly the files the classifier does not list. This script is
# the same scan with no toolchain and no classifier, in the lint job that
# runs on every pull request, so the skip loses nothing.
#
# The needles are assembled from parts so this file does not match itself.
# Cargo.lock is not read, as in the Rust test.
set -euo pipefail
export LC_ALL=C

hits="$(git grep -n -I -F -e "ai-""context" -e "standards""/" -- . ':!Cargo.lock' || true)"
if [ -n "$hits" ]; then
	echo "$hits" | while IFS= read -r line; do
		file="${line%%:*}"
		rest="${line#*:}"
		lineno="${rest%%:*}"
		echo "::error file=${file},line=${lineno}::cites a document tree that is not in this repository; say what the code does or pins instead"
	done
	echo "$hits"
	exit 1
fi
echo "no references to a private document tree in tracked files"
