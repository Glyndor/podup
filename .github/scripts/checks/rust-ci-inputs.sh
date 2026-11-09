#!/usr/bin/env bash
#
# Usage: rust-ci-inputs.sh BASE_SHA HEAD_SHA
#
# Decide whether the Rust CI lanes have to run for a pull request's range.
#
# The Rust lanes are `rust / Format & lint`, `rust / Test`,
# `rust / Extra platforms` and `rust / Coverage`: the format and lint pass,
# the Linux test suite, the macOS and Windows matrix, and the coverage gate.
# Together they are about ten minutes of runner time on every pull request,
# and a pull request that touches none of their inputs pays all of it for
# nothing.
#
# Reads $EVENT (the GitHub event name) and writes `run=true` or `run=false`
# to $GITHUB_OUTPUT. Exit 0 means the lanes may be skipped, exit 1 means they
# must run. The workflow reads the OUTPUT rather than the exit code, because
# a missing output has to mean "run" and never "skip": the classifier is the
# only thing standing between a broken checkout and a skipped test suite.
#
# The workflow runs this script from the reviewed base commit, never from the
# pull request's own tree, so a pull request cannot rewrite the judge of its
# own diff. Every path below is therefore reviewed policy, and every path
# that is not listed runs everything: a path nobody thought about is a path
# whose effect on the lanes nobody measured.
#
# What forces the lanes to run:
#
#   Cargo.toml, Cargo.lock      the dependency graph the lanes compile
#   internal/                   the crate itself
#   build.rs                    the crate's build script
#   tests/                      the Rust test suite and its fixtures
#   rustfmt.toml, .cargo/       what fmt and clippy are told to do
#   debian/                     the packaging the tests assert on
#   .github/workflows/          what CI runs
#   .github/scripts/            the checks CI runs, this script among them
#   README.md, CONTRIBUTING.md  read by the repository's own tests
#   docs/*.md                   read by the repository's own tests
#   install.sh, install.ps1     read by the release-key test
#   deny.toml                   read by the supply-chain test
#   fuzz/, bench/               read by the repository's own tests
#   .github/dependabot.yml      read by the supply-chain test
#
# What may be skipped is what no Rust lane and no Rust test needs to see: the
# licence, the editor settings, the cargo-about licence templates, the issue
# templates, the images under docs/assets, and the shell test suite under
# tests/scripts, which the shell lane owns.
#
# One Rust test does read those files: `no_private_document_references` lists
# every tracked file with `git ls-files` and reads it, the skippable ones
# included. The skip is safe only because `lint` runs the same scan as
# `check-private-document-references.sh` on every pull request, ungated, in
# `reusable-rust-ci.yml`. Take that step away and these rules become holes.
#
# The documentation is on the list above, and that is not an oversight:
# `tests/repository/docs/docs_links.rs` reads every markdown file under
# docs/ and `tests/repository/packaging/podup_is_not_published.rs` reads them
# too, so a change to any of them has to run the tests that read them.
set -uo pipefail
export LC_ALL=C

base="${1:-}"
head="${2:-}"
output="${GITHUB_OUTPUT:-}"

answer() { # <true|false> <reason>
	if [ -n "$output" ]; then
		echo "run=$1" >>"$output"
	fi
	echo "$2"
	if [ "$1" = false ]; then
		exit 0
	fi
	exit 1
}

if [ -z "$base" ] || [ -z "$head" ]; then
	answer true "no range to classify; the Rust lanes run"
fi
case "${EVENT:-}" in
	pull_request | push | merge_group) ;;
	*) answer true "event ${EVENT:-unknown} cannot skip; the Rust lanes run" ;;
esac
changed="$(git diff --name-only --no-renames "$base" "$head" 2>/dev/null)" ||
	answer true "cannot read $base..$head; the Rust lanes run"
[ -n "$changed" ] || answer true "the range is empty; the Rust lanes run"

while IFS= read -r path; do
	case "$path" in
		# The judge itself: a pull request that edits this script must not
		# be judged by its own edit.
		.github/scripts/checks/rust-ci-inputs.sh) answer true "$path changes the classifier" ;;
		# What the lanes compile.
		Cargo.* | */Cargo.*) answer true "$path changes the dependency graph" ;;
		internal/*) answer true "$path changes the crate" ;;
		build.rs) answer true "$path changes the build script" ;;
		rustfmt.toml | .cargo/*) answer true "$path changes what fmt and clippy do" ;;
		# The test suite and the fixtures it reads. The shell and PowerShell
		# scripts under tests/scripts are the shell lane's, and the only Rust
		# test that reads them scans for a private document tree, which `lint`
		# repeats outside this classifier; any other file there, a stray .rs
		# among them, is still walked by the Rust tests and so runs the lanes.
		tests/scripts/*.sh | tests/scripts/*.ps1) ;;
		tests/*) answer true "$path changes the test suite" ;;
		debian/*) answer true "$path changes the packaging" ;;
		# The CI itself.
		.github/workflows/*) answer true "$path changes the CI" ;;
		.github/scripts/*) answer true "$path changes the checks CI runs" ;;
		# Files the Rust tests read.
		README.md | CONTRIBUTING.md) answer true "$path is read by the repository tests" ;;
		docs/*.md) answer true "$path is read by the repository tests" ;;
		install.sh | install.ps1) answer true "$path is read by the release-key test" ;;
		deny.toml) answer true "$path is read by the supply-chain test" ;;
		fuzz/*) answer true "$path is read by the repository tests" ;;
		bench/*) answer true "$path is read by the repository tests" ;;
		.github/dependabot.yml) answer true "$path is read by the supply-chain test" ;;
		# What no Rust lane and no Rust test reads.
		LICENSE | .editorconfig | about.hbs | about.toml) ;;
		.github/CODEOWNERS | .github/PULL_REQUEST_TEMPLATE.md) ;;
		.github/ISSUE_TEMPLATE/*) ;;
		docs/assets/*) ;;
		*) answer true "$path is not a known input of the Rust lanes" ;;
	esac
done <<<"$changed"

answer false "nothing in this range can reach the Rust lanes"
