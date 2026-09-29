# shellcheck shell=bash
# Detect what a compose tool left behind on its engine after `down -v`.
#
# Sourced by bench/run.sh. After every timed `down` the harness asks the
# engine whether the tool actually left a clean host behind, records the row
# with rc=BENCH_RC_LEFTOVERS (97) instead of the tool's exit code when the
# tool exited 0 but left resources, prints one warning to stderr, and removes
# the leftovers. Without this, the published `scale` row compared one tool
# that actually tore the stack down against one that left four replicas
# running plus a pod and a network, and those leftovers stayed on the host
# and slowed every later row.
#
# Identification: by compose project label, not by name pattern. Every tool
# stamps each created resource with a label whose value is the project name;
# matching on the label is exact by construction and survives name-shape
# changes the tools make between versions (podman-compose 1.6.0 uses
# `<proj>_<service>_<n>` with underscores; podup and docker-compose use
# `<proj>-<service>-<n>` with hyphens; an old regex-based match had to know
# the separator per tool and could be tricked by a longer project that shared
# a prefix). The label keys are:
#
#   - podup:                       `podup.project`
#   - podman-compose 1.6.0:        `io.podman.compose.project`
#                                  (also sets `com.docker.compose.project`,
#                                  but podman-compose 1.6.0 sets both and the
#                                  podman one is the more specific marker)
#   - docker-compose (both engines): `com.docker.compose.project`
#
# The podman-compose pod (`pod_<proj>`) carries no label, so it is matched
# by exact name against the engine's full pod list. Nothing else in the
# suite has a label-less pod, and a stricter matcher (only that name, on
# every engine query) keeps a foreign pod out of the result.
#
# Engine selection mirrors what run.sh hands each tool. podup, podman-compose
# and docker-compose-podman drive Podman at `unix://$PODMAN_SOCK`, the same
# socket run.sh forces on them. docker-compose-docker drives dockerd at
# `unix://$DOCKER_DEFAULT_SOCKET`; the bench queries it through `docker -H`
# with that socket set explicitly, because plain `docker` inherits the
# caller's DOCKER_HOST and would hit a different daemon on hosts where both
# are running. PODMAN_SOCK and DOCKER_DEFAULT_SOCKET are exported by run.sh
# (and by engine.sh for the latter) before this file is sourced.
#
# Format fields: containers use `{{.Names}}` (plural), pods and networks
# use `{{.Name}}` (singular). The plural form is a runtime error on Podman
# 5.7 for pod/network listing and on dockerd's network listing; the real
# CLI returns rc=125 and the harness must treat an engine error as a hard
# failure (stderr message, non-zero exit), not as a clean engine.
#
# Requires: PODMAN_SOCK and DOCKER_DEFAULT_SOCKET set in scope; podman
# and/or docker on PATH.

# Fixed non-zero code for "the tool exited 0 but left resources behind".
# Picked as one no tool returns on its own merits: podup exits 0/1/2;
# podman-compose propagates podman's exit code (0/1/125/130); docker-compose
# 0/1/2. 97 is unused. aggregate.py treats any rc != 0 as a failure, so a
# code the tool itself never emits is enough to route these to the failure
# column without colliding with a tool error.
: "${BENCH_RC_LEFTOVERS:=97}"

# Compose-engine command the bench queries. Mirrors what run.sh hands each
# tool: docker-compose-docker against the docker CLI on the default socket,
# every other tool against Podman at $PODMAN_SOCK. The docker invocation
# carries `-H` so it does not inherit a caller's DOCKER_HOST: run.sh forces
# docker-compose-docker onto the same socket, and the cleanup queries must
# hit the same daemon.
bench_engine_command() { # tool
	local tool="$1"
	case "$tool" in
		docker-compose-docker) printf '%s\n' "docker -H unix://$DOCKER_DEFAULT_SOCKET" ;;
		podup|podman-compose|docker-compose-podman)
			printf '%s\n' "podman --url unix://$PODMAN_SOCK"
			;;
		*) return 2 ;;
	esac
}

# Compose project label key per tool. Returns 2 on an unknown tool; never
# prints an empty key.
bench_project_label() { # tool
	local tool="$1"
	case "$tool" in
		podup) printf '%s\n' "podup.project" ;;
		podman-compose) printf '%s\n' "io.podman.compose.project" ;;
		docker-compose-podman | docker-compose-docker) printf '%s\n' "com.docker.compose.project" ;;
		*) return 2 ;;
	esac
}

# Containers, pods and networks whose compose-project label (or pod name,
# for the podman-compose pod) belongs to <proj>. Names printed one per line,
# in the order: containers, then pods (podman-compose only), then networks.
#
# Exit codes:
#   0  - clean engine or leftovers listed on stdout
#   2  - unknown tool (programmer error; same code the rest of the bench
#        uses for "unreachable")
#   125 - engine query failed (network error, socket missing, format
#         rejection, label-filter rejection). Stderr already carries a
#         message; the caller must NOT treat this as "clean".
#
# Args: <tool> <project>
bench_leftovers() { # tool project
	local tool="$1" proj="$2" cmd label_key cnames pnames nnames

	cmd=$(bench_engine_command "$tool") || return 2
	label_key=$(bench_project_label "$tool") || return 2

	# Run all three queries first so a failure on one does not leave a
	# partial list on stdout (a partial list with rc=125 would be read as
	# "some leftovers, plus an error" and confuse the caller).
	cnames=$($cmd ps -a --filter "label=$label_key=$proj" --format '{{.Names}}') || {
		printf 'bench_leftovers: %s containers query failed\n' "$tool" >&2
		return 125
	}

	if [ "$tool" = podman-compose ]; then
		# The pod carries no label; match by exact name against the full
		# list. Nothing else in the suite produces a label-less pod.
		pnames=$(podman --url "unix://$PODMAN_SOCK" pod ls --format '{{.Name}}') || {
			printf 'bench_leftovers: podman-compose pods query failed\n' >&2
			return 125
		}
	else
		pnames=""
	fi

	nnames=$($cmd network ls --filter "label=$label_key=$proj" --format '{{.Name}}') || {
		printf 'bench_leftovers: %s networks query failed\n' "$tool" >&2
		return 125
	}

	# All three queries succeeded; emit the leftovers, if any. The empty
	# lines around $pnames are tolerated: `grep -xF` either matches the
	# pod name (one line) or nothing (zero lines), and `printf` already
	# added a separator after $cnames.
	if [ -n "$pnames" ]; then
		grep -xF "pod_${proj}" <<<"$pnames" || true
	fi
	printf '%s\n%s\n' "$cnames" "$nnames"
}

# Force-remove every leftover <proj> leaves behind. Idempotent: a clean
# engine is a no-op. Containers first (so the network and any pod they
# share can be removed), then the pod, then the network. Removal commands
# run with their stderr on the harness's stderr; a non-zero rc propagates
# so the caller can react.
#
# Stdout from the removal commands is captured and discarded. `podman rm`,
# `podman pod rm -f` and `podman network rm` all echo the ID of what they
# removed on Podman 5.7, which would leak into the harness's stdout if not
# captured. Stderr is kept on the harness: a real failure (container
# running, network in use, etc.) is information the operator needs to see.
#
# Args: <tool> <project>
bench_leftovers_purge() { # tool project
	local tool="$1" proj="$2" cmd label_key cids pids nids

	cmd=$(bench_engine_command "$tool") || return 2
	label_key=$(bench_project_label "$tool") || return 2

	# Containers. `--format '{{.ID}}'` is used instead of `-q` because
	# `podman ps -a -q` works for IDs but `podman network ls -q` returns
	# the network NAME on Podman 5.7 (a regression from older versions
	# where it returned the ID); the explicit format keeps both queries
	# consistent.
	cids=$($cmd ps -a --filter "label=$label_key=$proj" --format '{{.ID}}') || {
		printf 'bench_leftovers_purge: %s containers query failed\n' "$tool" >&2
		return 125
	}
	if [ -n "$cids" ]; then
		# shellcheck disable=SC2086
		$cmd rm -f $cids >/dev/null || {
			printf 'bench_leftovers_purge: %s container rm failed\n' "$tool" >&2
			return 125
		}
	fi

	if [ "$tool" = podman-compose ]; then
		pids=$(podman --url "unix://$PODMAN_SOCK" pod ls --filter "name=pod_${proj}" --format '{{.ID}}') || {
			printf 'bench_leftovers_purge: podman-compose pods query failed\n' >&2
			return 125
		}
		if [ -n "$pids" ]; then
			# shellcheck disable=SC2086
			podman --url "unix://$PODMAN_SOCK" pod rm -f $pids >/dev/null || {
				printf 'bench_leftovers_purge: podman-compose pod rm failed\n' >&2
				return 125
			}
		fi
	fi

	nids=$($cmd network ls --filter "label=$label_key=$proj" --format '{{.ID}}') || {
		printf 'bench_leftovers_purge: %s networks query failed\n' "$tool" >&2
		return 125
	}
	if [ -n "$nids" ]; then
		# shellcheck disable=SC2086
		$cmd network rm $nids >/dev/null || {
			printf 'bench_leftovers_purge: %s network rm failed\n' "$tool" >&2
			return 125
		}
	fi
	return 0
}

# Detect, warn on stderr, purge, and return BENCH_RC_LEFTOVERS when there
# were leftovers; return 0 on a clean engine; return 125 on an engine
# error. Used both after a timed `down` (where the caller overrides the
# row's rc when the function returns non-zero) and inside teardown (where
# only the warning matters; the caller ignores the rc).
#
# After the purge, the engine is re-queried: if anything of <proj> still
# answers, that is an error on stderr and a non-zero rc (so a partial
# cleanup is not silent).
#
# Args: <tool> <project>
bench_check_leftovers() { # tool project
	local tool="$1" proj="$2" leftovers after

	if ! leftovers=$(bench_leftovers "$tool" "$proj"); then
		# bench_leftovers already emitted the per-query stderr message;
		# nothing more useful to add here.
		return 125
	fi
	if [ -z "$leftovers" ]; then
		return 0
	fi

	# One warning line, naming the tool, the project (which doubles as the
	# scenario id) and the leftover names. The leftover list itself is
	# newline-separated (containers, then pods for podman-compose, then
	# networks); collapse it to space-separated so each warning in a long
	# run log is one grep-able line instead of one line per resource. The
	# next iteration needs a clean host whatever the tool did, so the
	# purge runs unconditionally.
	leftovers_one_line=${leftovers//$'\n'/ }
	printf 'warning: %s %s left behind after down: %s\n' \
		"$tool" "$proj" "$leftovers_one_line" >&2

	if ! bench_leftovers_purge "$tool" "$proj"; then
		printf 'error: %s %s purge failed (see stderr above)\n' \
			"$tool" "$proj" >&2
		return 125
	fi

	# Re-check: a failed purge must not read as "clean". An engine error on
	# the second query is reported the same way (stderr + non-zero).
	if ! after=$(bench_leftovers "$tool" "$proj"); then
		printf 'error: %s %s post-purge query failed (see stderr above)\n' \
			"$tool" "$proj" >&2
		return 125
	fi
	if [ -n "$after" ]; then
		printf 'error: %s %s still has leftovers after purge: %s\n' \
			"$tool" "$proj" "$after" >&2
		return 125
	fi

	return "$BENCH_RC_LEFTOVERS"
}

# Rewrite the rc field of a timeit output ("wall_s max_rss_kb cpu_s rc"),
# keeping the first three fields intact. Used to override the row when the
# tool exited 0 but left resources behind.
#
# Args: <timeit-out> <new-rc>
bench_override_rc() { # out new_rc
	local out="$1" new_rc="$2" w m c
	read -r w m c _ <<<"$out"
	printf '%s %s %s %s\n' "$w" "$m" "$c" "$new_rc"
}

# Decide which rc to record for a `down` row. The tool's own rc wins when
# it is non-zero: a tool that already failed does not need the harness to
# rewrite its row, and an `[F failed of N]` cell is published correctly
# either way. The 97 rewrite applies only when the tool reported success
# AND leftovers were found; recording 97 on top of a non-zero tool rc
# would mask the real failure.
#
# Args: <tool_rc> <leftover-text>
# Prints: rc to record
bench_record_rc() { # tool_rc leftover_text
	local tool_rc="$1" leftovers="$2"
	if [ -n "$leftovers" ] && [ "$tool_rc" = "0" ]; then
		printf '%s\n' "$BENCH_RC_LEFTOVERS"
	else
		printf '%s\n' "$tool_rc"
	fi
}
