#!/usr/bin/env bash
#
# Usage: build-seed.sh MODE OUT_DIR
# Writes OUT_DIR/user-data and OUT_DIR/seed.iso, the cloud-init seed of the VM of the
# Podman lane, from user-data.tpl next to this file.
#
# MODE is `suite` or `coverage`. They are two VMs with two budgets (.github/podman_lane/budget):
# the gating suite with its leak scan and the Quadlet generator tests, and the instrumented
# run that measures the coverage. Only the second installs llvm, which carries the coverage
# tools and nothing else uses.
set -euo pipefail
export LC_ALL=C

[ "$#" -eq 2 ] || {
	echo "usage: build-seed.sh suite|coverage OUT_DIR" >&2
	exit 2
}
mode="$1"
out="$2"
case "$mode" in
suite) llvm="" ;;
coverage) llvm="llvm" ;;
*)
	echo "build-seed.sh: the mode is suite or coverage, not '$mode'" >&2
	exit 2
	;;
esac
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mkdir -p "$out"
sed -e "s/__MODE__/$mode/" -e "s/__LLVM__/$llvm/" "$here/user-data.tpl" >"$out/user-data"
echo "VM seed for the $mode run of Podman"
cloud-localds "$out/seed.iso" "$out/user-data"
