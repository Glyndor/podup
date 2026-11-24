#!/usr/bin/env bash
#
# Usage: install-podman.sh
# Installs Podman on the native lane's runner from the Glyndor apt repository,
# whose source file the workflow has already written, and refuses a version
# below PODMAN_FLOOR (required, from the job's environment).
#
# What it guards against is the runner's own network, not the code under test:
# a stalled mirror used to kill `apt-get update` after two minutes, the next
# command ran anyway, and the job went on with the Podman the image ships
# (4.9.3) and failed far from the cause. Here:
#   - `update` reads Glyndor's source file only, so no Ubuntu mirror is asked
#     for anything this job does not need. Lists of the other sources stay on
#     disk (List-Cleanup=0), and `install` takes Podman's dependencies from
#     them. If that is not enough, a general refresh runs before the next try.
#   - the whole install is tried up to INSTALL_PODMAN_TRIES times with a wait
#     between tries. apt's own retries do not cover an external `timeout`
#     killing apt-get, so the loop is here.
#   - a failed install ends the script with a non-zero status and a message
#     that says so, and the caller's later steps are conditioned on it.
#
# Environment (all but PODMAN_FLOOR are for tests):
#   PODMAN_FLOOR           lowest accepted version, `6.2.0`
#   PHASES                 file that receives `podman=<version>` when set
#   INSTALL_PODMAN_SOURCE  the source file (default /etc/apt/sources.list.d/glyndor.sources)
#   INSTALL_PODMAN_TRIES   number of tries (default 3)
#   INSTALL_PODMAN_WAITS   seconds to wait after each failed try (default "15 45")
#   INSTALL_PODMAN_SUDO    the privilege wrapper (default sudo)
set -uo pipefail
export LC_ALL=C

fail() {
	echo "::error::$*" >&2
	exit 1
}

[ "$#" -eq 0 ] || fail "usage: install-podman.sh"
[ -n "${PODMAN_FLOOR:-}" ] || fail "PODMAN_FLOOR is not set"
source_file="${INSTALL_PODMAN_SOURCE:-/etc/apt/sources.list.d/glyndor.sources}"
tries="${INSTALL_PODMAN_TRIES:-3}"
case "$tries" in '' | *[!0-9]* | 0) fail "INSTALL_PODMAN_TRIES must be a positive number" ;; esac
read -ra waits <<<"${INSTALL_PODMAN_WAITS:-15 45}"
read -ra priv <<<"${INSTALL_PODMAN_SUDO-sudo}"
if [ ! -f "$source_file" ] || [ -L "$source_file" ]; then
	fail "$source_file is not a regular file"
fi

scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir "$scratch/parts"
cp "$source_file" "$scratch/parts/glyndor.sources"

export DEBIAN_FRONTEND=noninteractive
common=(-o Acquire::Retries=2 -o Acquire::http::Timeout=20 -o Acquire::https::Timeout=20 -o Acquire::ForceIPv4=true)
only_glyndor=(-o Dir::Etc::sourcelist=/dev/null -o "Dir::Etc::sourceparts=$scratch/parts" -o APT::Get::List-Cleanup=0)

apt() { # <seconds> <apt-get arguments...>
	local limit="$1"
	shift
	"${priv[@]}" timeout -k 10 "$limit" apt-get -q "${common[@]}" "$@"
}

attempt=1
while :; do
	updated=0
	if apt 90 "${only_glyndor[@]}" update; then
		updated=1
		apt 240 install -y podman && break
	fi
	if [ "$attempt" -ge "$tries" ]; then
		fail "Podman could not be installed after $tries tries; the runner could not reach the package sources, which says nothing about this change"
	fi
	echo "::warning::try $attempt of $tries to install Podman failed"
	# An interrupted install leaves dpkg half done, and the next try would stop on it.
	"${priv[@]}" timeout -k 10 120 dpkg --configure -a || true
	# The lists of Glyndor's repository were fine but the install still failed: a
	# dependency may be missing from the lists the image cached.
	if [ "$updated" -eq 1 ]; then
		apt 90 update || echo "::warning::the general package list refresh failed too"
	fi
	if [ "${#waits[@]}" -gt 0 ]; then
		idx=$((attempt - 1))
		[ "$idx" -lt "${#waits[@]}" ] || idx=$((${#waits[@]} - 1))
		sleep "${waits[$idx]}"
	fi
	attempt=$((attempt + 1))
done

ver="$(podman --version | awk '{print $3}')"
echo "podman: $ver"
if [ "$(printf '%s\n%s\n' "$ver" "$PODMAN_FLOOR" | sort -V | head -1)" != "$PODMAN_FLOOR" ]; then
	fail "the runner has Podman ${ver:-unknown}, below the $PODMAN_FLOOR floor podup requires"
fi
if [ -n "${PHASES:-}" ]; then
	echo "podman=$ver" >>"$PHASES"
fi
