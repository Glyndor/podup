#!/usr/bin/env bash
# Exit 0 only for a non-empty diff made entirely of paths that `cargo test` and
# `cargo llvm-cov` for the root workspace never read. This is a smaller set than
# rust-build-inputs.sh on purpose: the integration tests under tests/ are inputs
# here, and so is every file those tests open. Measured on the tree: the
# repository tests read README.md, CONTRIBUTING.md and docs/*.md (link and flag
# checks), install.sh and install.ps1 (pinned key copies), debian/*, fuzz/*,
# every workflow under .github/workflows, .github/dependabot.yml and
# .github/scripts/checks/check-branch-conclusion.sh. None of those is listed.
# Only what no test opens is: the licence text, the bench tree (apart from the
# manifest whose existence the dependabot test reads) and the GitHub community
# files. A path not named here is an input, and so is any new path.
set -uo pipefail
export LC_ALL=C

build() {
	echo "rust tests required: $*" >&2
	exit 1
}

[ "$#" -eq 1 ] || build "usage: rust-test-inputs.sh CHANGED_FILES"
[ -n "$1" ] || build "empty diff"
while IFS= read -r path; do
	case "$path" in
		bench/timeit/Cargo.toml) build "$path" ;;
		bench/* | LICENSE) ;;
		.github/CODEOWNERS | .github/ISSUE_TEMPLATE/* | .github/PULL_REQUEST_TEMPLATE.md) ;;
		*) build "$path" ;;
	esac
done <<< "$1"
echo "only paths the root tests never read changed"
