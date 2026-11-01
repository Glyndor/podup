#!/usr/bin/env bash
# Exit 0 only for a non-empty diff made entirely of paths that `cargo build`
# and `cargo doc` for the root package never read: the integration tests, the
# prose docs, the benchmark and fuzz workspaces (both excluded from the root
# workspace), the installers and the Debian packaging. Everything else, and
# every path not named here, is a build input. No glob over a Rust source
# directory is evidence that a file is unused, so internal/ is never listed.
set -uo pipefail
export LC_ALL=C

build() {
	echo "rust build required: $*" >&2
	exit 1
}

[ "$#" -eq 1 ] || build "usage: rust-build-inputs.sh CHANGED_FILES"
[ -n "$1" ] || build "empty diff"
while IFS= read -r path; do
	case "$path" in
		tests/* | docs/* | bench/* | fuzz/* | debian/* | \
		README.md | CONTRIBUTING.md | LICENSE | install.sh | install.ps1) ;;
		# Workflows and checks that never build, test, document or harden the root
		# package. Named one by one: a new workflow stays a build input until it is
		# reviewed and added here, and so do the toolchain callers (ci, reusable-rust-ci,
		# lint-shell, lint-powershell, release and the hardening and packaging ones).
		.github/dependabot.yml | .github/CODEOWNERS | .github/ISSUE_TEMPLATE/* | \
		.github/PULL_REQUEST_TEMPLATE.md) ;;
		.github/scripts/checks/rust-build-inputs.sh) build "$path" ;;
		.github/scripts/checks/*) ;;
		.github/workflows/audit.yml | .github/workflows/benchmark.yml | \
		.github/workflows/binary-budget.yml | .github/workflows/dco.yml | \
		.github/workflows/dependabot-security-watch.yml | .github/workflows/fuzz.yml | \
		.github/workflows/line-limit.yml | .github/workflows/main-guard.yml | \
		.github/workflows/pin-watch.yml | .github/workflows/podman-lane.yml | \
		.github/workflows/podman-lane-develop-nightly.yml | \
		.github/workflows/podman-watch.yml | .github/workflows/supply-chain.yml | \
		.github/workflows/workflow-lint.yml | \
		.github/workflows/reusable-branch-health.yml | .github/workflows/reusable-dco.yml | \
		.github/workflows/reusable-dependabot-freshness.yml | \
		.github/workflows/reusable-installer-contract.yml | \
		.github/workflows/reusable-line-limit.yml | \
		.github/workflows/reusable-main-guard.yml | \
		.github/workflows/reusable-rust-audit.yml | \
		.github/workflows/reusable-rust-debian.yml | \
		.github/workflows/reusable-rust-fuzz.yml | \
		.github/workflows/reusable-rust-supply-chain.yml | \
		.github/workflows/reusable-schedule-freshness.yml | \
		.github/workflows/reusable-workflow-lint.yml) ;;
		*) build "$path" ;;
	esac
done <<< "$1"
echo "only paths the root build and rustdoc never read changed"
