#!/usr/bin/env bash
#
# Usage: debian-ci-inputs.sh BASE_SHA HEAD_SHA
#
# Decide whether the Debian package has to be built for a pull request's range.
#
# The Debian lane builds the .deb with dpkg-buildpackage and, a second time,
# offline from vendored crates. It is the only lane that compiles with
# debian/rules' feature set (`--no-default-features --features
# watch,completions`), and the only one that runs the Rust test suite inside a
# package build: debian/rules overrides dh_auto_test with `cargo test`, and
# dpkg-buildpackage runs that sequence. Seven minutes of runner time for a
# change that cannot reach the package is time nobody gets back.
#
# Reads $EVENT (the GitHub event name) and writes `run=true` or `run=false`
# to $GITHUB_OUTPUT. Exit 0 means the package may be skipped, exit 1 means it
# must be built. The workflow reads the OUTPUT rather than the exit code,
# because a missing output has to mean "build" and never "skip": the
# classifier is the only thing standing between a broken checkout and a
# package nobody built.
#
# The workflow runs this script from the reviewed base commit, never from the
# pull request's own tree, so a pull request cannot rewrite the judge of its
# own diff. Every path below is therefore reviewed policy, and every path
# that is not listed builds the package: a path nobody thought about is a
# path whose effect on the package nobody measured.
#
# What forces the package to be built:
#
#   Cargo.toml, Cargo.lock      the dependency graph the package compiles
#   internal/                   the crate itself
#   build.rs                    the crate's build script
#   tests/                      the Rust test suite the package build runs
#   debian/                     the packaging
#   .github/workflows/          what CI runs
#   .github/scripts/            the checks CI runs, this script among them
#   install.sh, install.ps1     the installers
#
# What may be skipped is the shell and PowerShell test suite under
# tests/scripts: cargo compiles no .sh and no .ps1, debian/rules reads none of
# them, and the one Rust test that reads a tracked file whatever it is
# (no_private_document_references, which scans the tree for a private document
# tree) belongs to the Rust lane, which runs only when rust-ci-inputs.sh says
# the change is one of its inputs.
#
# The policy is deliberately narrower than what cannot reach the package. A
# licence, a README or an image under docs/assets cannot reach it either, and
# none of them may skip the build: the package is the artifact a Debian user
# installs, and the cost of building it for nothing is a few minutes of runner
# time next to the cost of shipping a package that was never built.
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
	answer true "no range to classify; the Debian package is built"
fi
case "${EVENT:-}" in
	pull_request | push | merge_group) ;;
	*) answer true "event ${EVENT:-unknown} cannot skip; the Debian package is built" ;;
esac
changed="$(git diff --name-only --no-renames "$base" "$head" 2>/dev/null)" ||
	answer true "cannot read $base..$head; the Debian package is built"
[ -n "$changed" ] || answer true "the range is empty; the Debian package is built"

while IFS= read -r path; do
	case "$path" in
		# The judge itself: a pull request that edits this script must not
		# be judged by its own edit.
		.github/scripts/checks/debian-ci-inputs.sh) answer true "$path changes the classifier" ;;
		# What the package compiles.
		Cargo.* | */Cargo.*) answer true "$path changes the dependency graph" ;;
		internal/*) answer true "$path changes the crate" ;;
		build.rs) answer true "$path changes the build script" ;;
		debian/*) answer true "$path changes the packaging" ;;
		# The test suite the package build runs. The shell and PowerShell
		# scripts under tests/scripts are not Rust, cargo compiles none of
		# them, and no Rust test reads one; any other file there is read by
		# the Rust tests, which the package build runs.
		tests/scripts/*.sh | tests/scripts/*.ps1) ;;
		tests/*) answer true "$path changes the Rust test suite" ;;
		# The CI itself.
		.github/workflows/*) answer true "$path changes the CI" ;;
		.github/scripts/*) answer true "$path changes the checks CI runs" ;;
		# The installers. The package build reads neither, but a change to
		# one is not a script-only test change.
		install.sh | install.ps1) answer true "$path changes an installer" ;;
		*) answer true "$path is not a script-only test change" ;;
	esac
done <<<"$changed"

answer false "nothing in this range can reach the Debian package"
