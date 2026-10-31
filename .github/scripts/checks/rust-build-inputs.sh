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
		*) build "$path" ;;
	esac
done <<< "$1"
echo "only paths the root build and rustdoc never read changed"
