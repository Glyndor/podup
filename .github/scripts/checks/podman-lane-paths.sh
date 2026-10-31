#!/usr/bin/env bash
#
# Usage: podman-lane-paths.sh CHANGED_FILES
# CHANGED_FILES is the non-empty, newline-separated git diff --name-only output.
# Exit 0 only when every path is outside the PR/merge-group VM execution.
# Other events always boot before this classifier is called. Unknown paths,
# malformed input and lane helpers fail closed. See INFORME-C1.md for evidence.
set -uo pipefail

no() {
	echo "reaches the VM: $*" >&2
	exit 1
}

[ "$#" -eq 1 ] || no "usage: podman-lane-paths.sh CHANGED_FILES"
[ -n "$1" ] || no "empty diff"
while IFS= read -r f; do
	case "$f" in
		.github/workflows/podman-lane.yml | .github/podman_lane/*) no "$f" ;;
		.github/scripts/release/sign-requirements.txt) ;;
		.github/scripts/*) no "$f" ;;
		*.md | docs/* | .github/ISSUE_TEMPLATE/* | LICENSE | .editorconfig) ;;
		.github/dependabot.yml | .github/CODEOWNERS) ;;
		fuzz/* | bench/* | tests/scripts/*) ;;
		deny.toml | about.toml | about.hbs | debian/* | install.sh | install.ps1) ;;
		# Explicit names keep future workflows closed until their callers are reviewed.
		.github/workflows/asset-contract.yml) ;;
		.github/workflows/audit.yml) ;;
		.github/workflows/benchmark.yml) ;;
		.github/workflows/binary-budget.yml) ;;
		.github/workflows/ci.yml) ;;
		.github/workflows/dco.yml) ;;
		.github/workflows/dependabot-security-watch.yml) ;;
		.github/workflows/debian-build.yml) ;;
		.github/workflows/fuzz.yml) ;;
		.github/workflows/line-limit.yml) ;;
		.github/workflows/lint-powershell.yml) ;;
		.github/workflows/lint-shell.yml) ;;
		.github/workflows/linux-hardening.yml) ;;
		.github/workflows/main-guard.yml) ;;
		.github/workflows/pin-watch.yml) ;;
		.github/workflows/podman-lane-develop-nightly.yml) ;;
		.github/workflows/podman-watch.yml) ;;
		.github/workflows/release.yml) ;;
		.github/workflows/reusable-branch-health.yml) ;;
		.github/workflows/reusable-dco.yml) ;;
		.github/workflows/reusable-dependabot-freshness.yml) ;;
		.github/workflows/reusable-installer-contract.yml) ;;
		.github/workflows/reusable-line-limit.yml) ;;
		.github/workflows/reusable-main-guard.yml) ;;
		.github/workflows/reusable-powershell-ci.yml) ;;
		.github/workflows/reusable-rust-audit.yml) ;;
		.github/workflows/reusable-rust-ci.yml) ;;
		.github/workflows/reusable-rust-debian.yml) ;;
		.github/workflows/reusable-rust-fuzz.yml) ;;
		.github/workflows/reusable-rust-supply-chain.yml) ;;
		.github/workflows/reusable-schedule-freshness.yml) ;;
		.github/workflows/reusable-shell-ci.yml) ;;
		.github/workflows/reusable-workflow-lint.yml) ;;
		.github/workflows/supply-chain.yml) ;;
		.github/workflows/windows-arm64.yml) ;;
		.github/workflows/workflow-lint.yml) ;;
		*) no "$f" ;;
	esac
done <<<"$1"

echo "nothing in this range can reach the VM"
