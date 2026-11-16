#!/usr/bin/env bash
#
# Usage: rust-ci-inputs.sh BASE_SHA HEAD_SHA
#
# Decide what the Rust CI lanes have to run for a pull request's range.
#
# The Rust lanes are `rust / Format & lint`, `rust / Test`,
# `rust / Extra platforms` and `rust / Coverage`: the format and lint pass,
# the Linux test suite, the macOS and Windows matrix, and the coverage gate.
#
# It answers with two outputs, and the pair is the answer:
#
#   run=true  docs_only=false   every lane runs
#   run=true  docs_only=true    only the focused documentation lane runs
#   run=false docs_only=false   no lane runs
#
# The third one is what a documentation-only pull request gets. The documents
# are read by a handful of tests and by nothing else, so running those tests
# is the whole of the work there is; the clippy pass, the macOS and Windows
# legs and the coverage gate are about the crate, and a change to a sentence
# cannot reach them.
#
# Exit 0 means no lane has to run, exit 1 means every lane has to run and
# exit 2 means the focused documentation lane has to run. The workflow reads
# the OUTPUT rather than the exit code, because a missing output has to mean
# "run" and never "skip": the classifier is the only thing standing between
# a broken checkout and a skipped test suite.
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
# The documentation is on the list above, and that is not an oversight:
# `tests/repository/docs/docs_links.rs` reads every markdown file under
# docs/ and `tests/repository/packaging/podup_is_not_published.rs` reads them
# too, so a change to any of them has to run the tests that read them.
#
# The focused lane runs the WHOLE set of tests that read a document, not only
# those two. `docs_flags` reads docs/commands.md and
# docs/commands-reference.md, `no_system_wide_apt_policy` and
# `keyring_bootstrap_order` read docs/debian-packaging.md, and the unit test
# in `internal/update/verify_key_copies_agree.rs` reads docs/self-update.md
# and fails when the document stops carrying the release key. The workflow
# names that set; this script only says which answer the range is.
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
set -uo pipefail
export LC_ALL=C

base="${1:-}"
head="${2:-}"
output="${GITHUB_OUTPUT:-}"

answer() { # <true|false> <true|false> <reason>
	if [ -n "$output" ]; then
		echo "run=$1" >>"$output"
		echo "docs_only=$2" >>"$output"
	fi
	echo "$3"
	if [ "$1" = false ]; then
		exit 0
	fi
	if [ "$2" = true ]; then
		exit 2
	fi
	exit 1
}

if [ -z "$base" ] || [ -z "$head" ]; then
	answer true false "no range to classify; the Rust lanes run"
fi
case "${EVENT:-}" in
	pull_request | push | merge_group) ;;
	*) answer true false "event ${EVENT:-unknown} cannot skip; the Rust lanes run" ;;
esac
changed="$(git diff --name-only --no-renames "$base" "$head" 2>/dev/null)" ||
	answer true false "cannot read $base..$head; the Rust lanes run"
[ -n "$changed" ] || answer true false "the range is empty; the Rust lanes run"

# A document whose name Windows cannot check out: CON, PRN, AUX, NUL, COM1-9
# and LPT1-9, with any extension, in any directory. Git for Windows refuses
# them, and the focused lane skips the Windows leg that would have said so.
windows_reserved() { # <path>
	local part stem rest="$1"
	while [ -n "$rest" ]; do
		part="${rest%%/*}"
		case "$rest" in */*) rest="${rest#*/}" ;; *) rest="" ;; esac
		stem="${part%%.*}"
		case "${stem^^}" in
			CON | PRN | AUX | NUL | COM[1-9] | LPT[1-9]) return 0 ;;
		esac
	done
	return 1
}

# Whether the range touches a document, and whether it touches anything else
# that no Rust lane reads. Documents alone are the focused lane; a document
# next to anything else runs every lane, because the two answers differ in
# what they skip and the safe reading of a mixed range is the whole thing.
docs=false
other=false

while IFS= read -r path; do
	case "$path" in
		# The judge itself: a pull request that edits this script must not
		# be judged by its own edit.
		.github/scripts/checks/rust-ci-inputs.sh) answer true false "$path changes the classifier" ;;
		# What the lanes compile.
		Cargo.* | */Cargo.*) answer true false "$path changes the dependency graph" ;;
		internal/*) answer true false "$path changes the crate" ;;
		build.rs) answer true false "$path changes the build script" ;;
		rustfmt.toml | .cargo/*) answer true false "$path changes what fmt and clippy do" ;;
		# The test suite and the fixtures it reads. The shell and PowerShell
		# scripts under tests/scripts are the shell lane's, and the only Rust
		# test that reads them scans for a private document tree, which `lint`
		# repeats outside this classifier; any other file there, a stray .rs
		# among them, is still walked by the Rust tests and so runs the lanes.
		tests/scripts/*.sh | tests/scripts/*.ps1) other=true ;;
		tests/*) answer true false "$path changes the test suite" ;;
		debian/*) answer true false "$path changes the packaging" ;;
		# The CI itself.
		.github/workflows/*) answer true false "$path changes the CI" ;;
		.github/scripts/*) answer true false "$path changes the checks CI runs" ;;
		# The documents. They are read by a few tests and by nothing else, so
		# a range of documents alone is the focused lane.
		README.md | CONTRIBUTING.md) docs=true ;;
		docs/*.md)
			windows_reserved "$path" && answer true false "$path has a name Windows cannot check out; every lane runs"
			docs=true
			;;
		install.sh | install.ps1) answer true false "$path is read by the release-key test" ;;
		deny.toml) answer true false "$path is read by the supply-chain test" ;;
		fuzz/*) answer true false "$path is read by the repository tests" ;;
		bench/*) answer true false "$path is read by the repository tests" ;;
		.github/dependabot.yml) answer true false "$path is read by the supply-chain test" ;;
		# What no Rust lane and no Rust test reads. These may be skipped, and
		# they are not documents either: a document next to one of them is a
		# mixed range and runs everything.
		LICENSE | .editorconfig | about.hbs | about.toml) other=true ;;
		.github/CODEOWNERS | .github/PULL_REQUEST_TEMPLATE.md) other=true ;;
		.github/ISSUE_TEMPLATE/*) other=true ;;
		# The images under docs/assets are not documents the tests read, and
		# they may sit next to one that is: the assets are skipped as they
		# always were, and the document decides the lane. A markdown file
		# under docs/assets is read by the same tests as any other document,
		# so the case above claims it first.
		docs/assets/*) ;;
		*) answer true false "$path is not a known input of the Rust lanes" ;;
	esac
done <<<"$changed"

if [ "$docs" = true ]; then
	if [ "$other" = false ]; then
		answer true true "only documentation changed; the focused documentation lane runs"
	fi
	answer true false "the range changes documentation and something else; every lane runs"
fi
answer false false "nothing in this range can reach the Rust lanes"
