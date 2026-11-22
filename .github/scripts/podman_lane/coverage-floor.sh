#!/usr/bin/env bash
#
# Usage: coverage-floor.sh PODMAN_VERSION EVENT COVERAGE
# PODMAN_VERSION is the full version the VM booted (`6.2.0`), EVENT the
# GitHub event name and COVERAGE the figure the VM printed (`91.5%`).
#
# The nightly coverage floor applies to the scheduled run on Podman 6, the
# major the lane exercises. The version arrives complete, so the major is
# taken from it before comparing: comparing `6.2.0` with `6` never matched
# and the floor was never applied.
#
# Exit 0 when the floor does not apply or the figure clears it; exit 1 when
# the figure is below it, is missing or cannot be read.
set -uo pipefail
export LC_ALL=C

FLOOR=88
GATED_MAJOR=6

[ "$#" -eq 3 ] || {
	echo "usage: coverage-floor.sh PODMAN_VERSION EVENT COVERAGE" >&2
	exit 1
}
ver="$1"
event="$2"
cov="$3"

major="${ver%%.*}"
if [ "$major" != "$GATED_MAJOR" ] || [ "$event" != "schedule" ]; then
	exit 0
fi

# The VM talks over a serial console, so the figure can arrive CRLF-terminated.
cov="${cov//$'\r'/}"
# A scheduled Podman 6 run measures coverage: no figure means the measurement
# did not finish (the VM timed out, say), which is a failure, not a pass.
if [ -z "$cov" ]; then
	echo "::error::no Podman $GATED_MAJOR coverage figure was reported; the nightly floor cannot be checked."
	exit 1
fi

# Strip the trailing `%` and round (the lane prints `91.5` for `91.5%`).
num="${cov%%%}"
if ! [[ "$num" =~ ^[0-9]+(\.[0-9]+)?$ ]]; then
	echo "::error::Podman $GATED_MAJOR coverage '$cov' is not a percentage; the nightly floor cannot be checked."
	exit 1
fi
cov_int="$(printf '%.0f' "$num")"
if [ "$cov_int" -lt "$FLOOR" ]; then
	echo "::error::Podman $GATED_MAJOR coverage $cov is below the ${FLOOR}% nightly floor (#1326). The full suite measured $cov, likely a regression that the floor is here to catch."
	exit 1
fi
