# shellcheck shell=bash
# Record the host environment next to the benchmark rows.
#
# write_env FILE writes one `key: value` line per datum the report needs to
# reproduce a published number. Sourced by run.sh right after pre-pull, and
# parsed by aggregate.py, which embeds the file at the top of report.md.
#
# Every datum is a single line, no continuation, no quoting: the file travels
# with raw.csv and is meant to stay diffable. `key: value` rather than YAML so
# the parser is ten lines and never gets in the way.
#
# The values are honest about being missing: a command not installed prints
# `absent`, a /sys node not there prints `unknown`. A reader can tell the
# difference between "the test runner had no virsh" and "we did not check".

write_env() { # FILE
	local f="$1"
	{
		# Time first: UTC ISO 8601 so the file sorts chronologically and
		# the wall clock matches the rows in raw.csv. localtime() would sort
		# too but would not survive sharing across timezones.
		printf 'date: %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
		printf 'kernel: %s\n' "$(uname -r)"
		# model name may have spaces; the value of cpuinfo is tab-prefixed on
		# Linux, which `cut -d: -f2-` strips. -m1: only one record is enough.
		printf 'cpu: %s\n' "$(grep -m1 '^model name' /proc/cpuinfo | cut -d: -f2- | sed 's/^ *//')"
		printf 'cores: %s\n' "$(nproc)"
		if [ -r /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor ]; then
			printf 'governor: %s\n' "$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor)"
		else
			printf 'governor: unknown\n'
		fi
		if [ -n "${CORES:-}" ]; then
			printf 'pinned_cores: %s\n' "$CORES"
		else
			printf 'pinned_cores: none\n'
		fi
		# Tool versions. `absent` when missing so the report says so and the
		# reader can correlate with a "X not installed" note from run.sh.
		if command -v "$PODUP_BIN" >/dev/null 2>&1; then
			# podup --version prints a leading blank line; grep -v drops it so
			# the value in env.txt stays one line.
			printf 'podup: %s\n' "$("$PODUP_BIN" --version | grep -v '^$' | head -1)"
			# file -b on the podup binary: "static-pie linked" on the musl
			# asset, "statically linked" on the plain musl build, "dynamically
			# linked" on any glibc cargo build --release. Anything else (a
			# script, a symlink) is reported verbatim.
			case "$(file -b "$PODUP_BIN")" in
				*"static-pie linked"*) printf 'podup_linkage: static-pie linked\n' ;;
				*"statically linked"*) printf 'podup_linkage: statically linked\n' ;;
				*"dynamically linked"*) printf 'podup_linkage: dynamically linked\n' ;;
				*) printf 'podup_linkage: %s\n' "$(file -b "$PODUP_BIN")" ;;
			esac
		else
			printf 'podup: absent\n'
			printf 'podup_linkage: absent\n'
		fi
		if command -v podman >/dev/null 2>&1; then
			printf 'podman: %s\n' "$(podman --version)"
		else
			printf 'podman: absent\n'
		fi
		if command -v podman-compose >/dev/null 2>&1; then
			# It prints its own version and then Podman's, in either order.
			printf 'podman_compose: %s\n' "$(podman-compose --version 2>&1 | grep -m1 '^podman-compose')"
		else
			printf 'podman_compose: absent\n'
		fi
		if command -v docker-compose >/dev/null 2>&1; then
			printf 'docker_compose: %s\n' "$(docker-compose version --short)"
		else
			printf 'docker_compose: absent\n'
		fi
		if command -v docker >/dev/null 2>&1; then
			printf 'docker_server: %s\n' "$(docker version --format '{{.Server.Version}}')"
		else
			printf 'docker_server: absent\n'
		fi
		# Engine state. Every counter uses `podman ... -q | wc -l` (or the
		# docker equivalent): quiet ID listing, line count is the cardinality.
		# -q keeps the count reliable; without it `podman ps` would print
		# headers or wrap on long IDs and the count would be off by one.
		# Containers: `-a` counts every container (stopped too), not just
		# running; a published run is over a fresh engine, so leftover rows
		# would mean the harness leaked something.
		if command -v podman >/dev/null 2>&1; then
			printf 'podman_containers: %s\n' "$(podman ps -aq 2>/dev/null | wc -l)"
			printf 'podman_networks: %s\n' "$(podman network ls -q 2>/dev/null | wc -l)"
			printf 'podman_volumes: %s\n' "$(podman volume ls -q 2>/dev/null | wc -l)"
			printf 'podman_images: %s\n' "$(podman images -q 2>/dev/null | wc -l)"
			printf 'podman_dangling_images: %s\n' "$(podman images -f dangling=true -q 2>/dev/null | wc -l)"
		else
			printf 'podman_containers: absent\n'
			printf 'podman_networks: absent\n'
			printf 'podman_volumes: absent\n'
			printf 'podman_images: absent\n'
			printf 'podman_dangling_images: absent\n'
		fi
		if command -v docker >/dev/null 2>&1; then
			printf 'docker_containers: %s\n' "$(docker ps -aq 2>/dev/null | wc -l)"
			printf 'docker_images: %s\n' "$(docker images -q 2>/dev/null | wc -l)"
		else
			printf 'docker_containers: absent\n'
			printf 'docker_images: absent\n'
		fi
		# virsh -c qemu:///system names every running domain on the host.
		# A non-empty count means another VM is sharing the same physical
		# cores we are about to pin the benchmark to, which makes the
		# published numbers noisy without saying so.
		if command -v virsh >/dev/null 2>&1; then
			printf 'running_vms: %s\n' "$(virsh -c qemu:///system list --state-running --name 2>/dev/null | grep -c .)"
		else
			printf 'running_vms: unknown\n'
		fi
	} > "$f"
}
