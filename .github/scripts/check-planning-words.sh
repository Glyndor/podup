#!/usr/bin/env bash
# Fail when a tracked file names a planning note instead of stating a fact.
#
# Comments that said "the brief calls this out" or "the cases the brief
# names" crept into nine files across the tree, in Rust, a workflow and a
# shell test. Each one pointed a reader at a document that is not in the
# repository, so the sentence explained nothing to anyone who opened the
# file. A review checklist looked for the word and still missed all nine:
# a rule that lives in prose is mostly followed, and this one was not.
#
# The match is the whole word, case-insensitive. The changelog is exempt
# because it records history in its own register.
set -euo pipefail

hits="$(git grep -n -I -i -w -E 'brief|briefs|briefing' -- ':!CHANGELOG.md' ':!.github/scripts/check-planning-words.sh' || true)"
if [ -n "$hits" ]; then
	echo "$hits" | while IFS= read -r line; do
		file="${line%%:*}"
		rest="${line#*:}"
		lineno="${rest%%:*}"
		echo "::error file=${file},line=${lineno}::names a planning note; say what the code does or pins instead"
	done
	echo "$hits"
	exit 1
fi
echo "no planning-note references in tracked files"
