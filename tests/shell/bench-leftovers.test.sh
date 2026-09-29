#!/usr/bin/env bash
# shellcheck shell=bash
#
# bench_leftovers() / bench_leftovers_purge() correctly enumerate and remove
# the containers, pods and networks a tool left behind on its engine after
# `down -v`.
#
# Why this exists: podman-compose 1.6.0's `down` exits 0 in about 0.38 s
# and leaves four replicas plus the pod and the network behind. run.sh
# trusted the exit code, so the published `scale` row compared one tool
# that actually tore the stack down against one that did a fifth of it,
# and the leftovers stayed on the host and slowed every later row.
#
# The match is on the compose-project label each tool stamps on every
# resource (`podup.project`, `io.podman.compose.project`,
# `com.docker.compose.project`), which makes it exact by construction and
# immune to the name-shape differences between tools (podman-compose uses
# underscores, podup and docker-compose use hyphens). The podman-compose
# pod `pod_<proj>` carries no label and is matched by exact name.
#
# The stub replaces both podman and docker on PATH. It mimics the real CLIs
# where it matters:
#
#   - `pod ls --format '{{.Names}}'` and `network ls --format '{{.Names}}'`
#     exit non-zero with the same error message the real CLI emits, so a
#     regression to the plural field fails the test instead of reading as
#     "clean";
#   - label-filtered queries (`--filter label=<key>=<value>`) answer only
#     for the requested label and value, so a foreign project with a
#     different label is neither reported nor removed;
#   - every `rm` / `pod rm` / `network rm` call is appended to a log file
#     with its arguments, so the tests can assert on which removal calls
#     the purge issued.
#
# Requires: bash, awk, sed, the sourced file, nothing else.
set -u

cd "$(dirname "$0")/../.." || exit 1
HERE="$PWD"

pass=0
fail=0

check() { # <description> <expected> <actual>
	if [ "$2" = "$3" ]; then
		echo "ok    $1"
		pass=$((pass + 1))
	else
		echo "FAIL  $1"
		echo "        expected: $2"
		echo "        actual:   $3"
		fail=$((fail + 1))
	fi
}

# Single stub at $STUBDIR/bin/_stub.sh, symlinked as both `podman` and
# `docker`. Each invocation is classified by walking the args looking for
# the subcommand; flag values (the URL/H/context arg that follows --url,
# -H, etc.) are skipped so a leading `--url unix://...` does not throw off
# the classification.
STUBDIR="$(mktemp -d)"
trap 'rm -rf "$STUBDIR"' EXIT
mkdir -p "$STUBDIR/bin"
STUBLOG="$STUBDIR/calls.log"
: > "$STUBLOG"
cat > "$STUBDIR/bin/_stub.sh" <<'STUB'
#!/usr/bin/env bash
# State files live next to the stub so that removal calls update the same
# store subsequent queries read from. $BENCH_STUB_STATE_DIR defaults to
# the directory containing the stub itself; the test exports it.
state_dir="${BENCH_STUB_STATE_DIR:-$(dirname "$STUBLOG")}"
log="${BENCH_STUB_LOG:-$STUBLOG}"

# Subcommand classification. Walk the args once and recognise the six
# subcommands the bench calls. "pod" / "network" alone are not subcommands
# (they pair with "ls" / "rm"); "ps" and "rm" are subcommands on their own.
subcmd=""
prev=""
skip_next=0
for arg in $@; do
	if [ "$skip_next" = 1 ]; then skip_next=0; prev=""; continue; fi
	case "$prev $arg" in
		"pod ls") subcmd=pod_ls; break ;;
		"pod rm") subcmd=pod_rm; break ;;
		"network ls") subcmd=network_ls; break ;;
		"network rm") subcmd=network_rm; break ;;
	esac
	if [ "$prev" != pod ] && [ "$prev" != network ]; then
		case "$arg" in
			ps) subcmd=ps; break ;;
			rm) subcmd=rm; break ;;
		esac
	fi
	case "$arg" in
		--url|-H|--host|--config|--context|--log-level) skip_next=1 ;;
	esac
	case "$arg" in
		pod|network) prev="$arg" ;;
		*) prev="" ;;
	esac
done

# Always log the full invocation: the Docker-explicit-host assertion reads
# from here, and a misrouted call shows up before the case statement does.
printf 'INVOKE %s\n' "$*" >> "$log"

# Extract --filter label=<key>=<value> from args. Returns "<key> <value>"
# or empty if not present. Key may contain dots; value does not.
filter_arg=$(printf '%s' "$*" | sed -n 's/.*--filter label=\([^=]*\)=\([^ ]*\).*/\1 \2/p')
label_key=$(printf '%s' "$filter_arg" | awk '{print $1}')
filter_value=$(printf '%s' "$filter_arg" | awk '{print $2}')
sanitized=$(printf '%s' "$label_key" | tr '.' '_')

# Read the state file for this label key + kind, filtering by value.
# Returns "" if the state file does not exist.
read_state() {
	local key_kind="$1" want="$2" mode="$3"
	local f="$state_dir/BENCH_STUB_BY_LABEL_${key_kind}"
	[ -f "$f" ] || return 0
	awk -F'\t' -v want="$want" -v mode="$mode" '
		$1 == want {
			if (mode == "id_only") print $3
			else print $2
		}' "$f"
}

# Remove rows from a state file whose column 3 (id) is in the id set.
purge_ids() {
	local key_kind="$1"
	shift
	local ids_str="$*"
	local f="$state_dir/BENCH_STUB_BY_LABEL_${key_kind}"
	[ -f "$f" ] || return 0
	awk -F'\t' -v ids="$ids_str" '
		BEGIN { n = split(ids, arr, " "); for (i=1; i<=n; i++) rm[arr[i]] = 1 }
		!($3 in rm) { print }' "$f" > "$f.tmp" && mv "$f.tmp" "$f"
}

case "$subcmd" in
	pod_ls)
		if [[ "$*" == *'{{.Names}}'* ]]; then
			printf 'Error: template: ls:1:13: executing "ls" at <.Names>: cant evaluate field Names in type pod.ListPrintReports\n' >&2
			exit 125
		fi
		pods_file="$state_dir/BENCH_STUB_PODS"
		if [[ "$*" == *'{{.ID}}'* ]]; then
			i=0
			if [ -f "$pods_file" ]; then
				while IFS= read -r n; do
					[ -z "$n" ] && continue
					printf 'pidx%d\n' "$i"
					i=$((i + 1))
				done < "$pods_file"
			fi
		else
			[ -f "$pods_file" ] && cat "$pods_file"
		fi
		;;
	network_ls)
		if [[ "$*" == *'{{.Names}}'* ]]; then
			printf 'Error: template: ls:1:13: executing "ls" at <.Names>: cant evaluate field Names in type network.ListPrintReports\n' >&2
			exit 125
		fi
		# Real engine's `-q` returns names on Podman 5.7, so the bench
		# uses `--format '{{.ID}}'` instead and falls back to the name
		# format otherwise.
		if [[ "$*" == *'{{.ID}}'* ]]; then
			mode=id_only
		else
			mode=name_only
		fi
		# No --filter label=... means return every row in the file, so
		# a sabotage that drops the filter shows up as a foreign-project
		# leak (the foreign rows are returned alongside the project's).
		if [ -z "$filter_value" ]; then
			f="$state_dir/BENCH_STUB_BY_LABEL_${sanitized}_NETWORKS"
			if [ -f "$f" ]; then
				if [ "$mode" = id_only ]; then
					awk -F'\t' '{print $3}' "$f"
				else
					awk -F'\t' '{print $2}' "$f"
				fi
			fi
		else
			read_state "${sanitized}_NETWORKS" "$filter_value" "$mode"
		fi
		;;
	ps)
		if [ "${BENCH_STUB_FAIL_PS:-0}" = 1 ]; then
			printf 'Error: stub: podman ps failed (injected)\n' >&2
			exit 125
		fi
		if [[ "$*" == *'{{.ID}}'* ]]; then
			mode=id_only
		else
			mode=name_only
		fi
		if [ -z "$filter_value" ]; then
			# No --filter label=...: return every container row.
			for f in "$state_dir"/BENCH_STUB_BY_LABEL_*_CONTAINERS; do
				[ -f "$f" ] || continue
				if [ "$mode" = id_only ]; then
					awk -F'\t' '{print $3}' "$f"
				else
					awk -F'\t' '{print $2}' "$f"
				fi
			done
		else
			read_state "${sanitized}_CONTAINERS" "$filter_value" "$mode"
		fi
		;;
	rm)
		# Real `podman rm -f` echoes each ID it removes on stdout, which
		# the bench captures (>/dev/null) so the warning stays names-only.
		# Extract IDs (everything after -f, until end of args).
		ids=$(printf '%s' "$*" | sed -n 's/.*-f //p')
		if [ -n "$ids" ]; then
			# Remove from every container state file (we do not know which
			# label key the IDs came from; purge across all).
			for f in "$state_dir"/BENCH_STUB_BY_LABEL_*_CONTAINERS; do
				[ -f "$f" ] || continue
				awk -F'\t' -v ids="$ids" '
					BEGIN { n = split(ids, arr, " "); for (i=1; i<=n; i++) rm[arr[i]] = 1 }
					!($3 in rm) { print }' "$f" > "$f.tmp" && mv "$f.tmp" "$f"
			done
		fi
		printf 'rm %s\n' "$*" >> "$log"
		;;
	pod_rm)
		# Pod rm operates on pods, not label-keyed. For simplicity, drop
		# the matching pod names from the pods file.
		ids=$(printf '%s' "$*" | sed -n 's/.*-f //p')
		if [ -n "$ids" ] && [ -f "$state_dir/BENCH_STUB_PODS" ]; then
			# ids are pidx0, pidx1, ...; the pods file has pod names
			# one per line, in the same order. Map idx -> name then drop.
			i=0
			to_drop=""
			while IFS= read -r n; do
				[ -z "$n" ] && continue
				case " $ids " in
					*" pidx$i "*) to_drop="$to_drop$n\n" ;;
				esac
				i=$((i + 1))
			done < "$state_dir/BENCH_STUB_PODS"
			if [ -n "$to_drop" ]; then
				grep -vF -e "$(printf '%s' "$to_drop" | head -n1)" "$state_dir/BENCH_STUB_PODS" > "$state_dir/BENCH_STUB_PODS.tmp" || true
				mv "$state_dir/BENCH_STUB_PODS.tmp" "$state_dir/BENCH_STUB_PODS"
			fi
		fi
		printf 'pod_rm %s\n' "$*" >> "$log"
		;;
	network_rm)
		# Real `podman network rm` echoes the network ID on stdout, which
		# the bench captures to /dev/null.
		ids=$(printf '%s' "$*" | sed -n 's/.* //p')
		[ -z "$ids" ] && ids=$(printf '%s' "$*" | sed -n 's/network rm //p')
		if [ -n "$ids" ]; then
			for f in "$state_dir"/BENCH_STUB_BY_LABEL_*_NETWORKS; do
				[ -f "$f" ] || continue
				awk -F'\t' -v ids="$ids" '
					BEGIN { n = split(ids, arr, " "); for (i=1; i<=n; i++) rm[arr[i]] = 1 }
					!($3 in rm) { print }' "$f" > "$f.tmp" && mv "$f.tmp" "$f"
			done
		fi
		printf 'network_rm %s\n' "$*" >> "$log"
		;;
	*)
		printf 'stub: unhandled invocation: %s\n' "$*" >&2
		exit 2
		;;
esac
STUB
chmod +x "$STUBDIR/bin/_stub.sh"
ln -s "$STUBDIR/bin/_stub.sh" "$STUBDIR/bin/podman"
ln -s "$STUBDIR/bin/_stub.sh" "$STUBDIR/bin/docker"

# load_table <kind> <label_key> <proj> <name> <id> [<proj> <name> <id> ...]
#
# Writes a tab-separated state file at
# $BENCH_STUB_STATE_DIR/BENCH_STUB_BY_LABEL_<sanitized-key>_<KIND>. The
# stub reads from the same directory and updates the file when an rm call
# fires, so post-purge queries return empty. Rows are
# <proj>\t<name>\t<id>; column 1 is the value the stub matches against
# `--filter label=<key>=<value>`.
load_table() { # kind key proj name id ...
	local kind="$1" key="$2"
	shift 2
	local sanitized
	sanitized=$(printf '%s' "$key" | tr '.' '_')
	local file="${BENCH_STUB_STATE_DIR:-$STUBDIR}/BENCH_STUB_BY_LABEL_${sanitized}_${kind}"
	: > "$file"
	while [ $# -ge 3 ]; do
		local proj="$1" name="$2" id="$3"
		shift 3
		printf '%s\t%s\t%s\n' "$proj" "$name" "$id" >> "$file"
	done
}

# load_pods <name> [<name> ...]
#
# Writes the pod names one per line to the BENCH_STUB_PODS file. The stub
# returns them with pidx<N> IDs (matching the order they were declared);
# pod rm removes the corresponding line.
load_pods() {
	local file="${BENCH_STUB_STATE_DIR:-$STUBDIR}/BENCH_STUB_PODS"
	: > "$file"
	for n in "$@"; do
		printf '%s\n' "$n" >> "$file"
	done
}

# clear_state
#
# Removes every state file in $BENCH_STUB_STATE_DIR so the next test case
# starts from a clean engine. Called at the start of each case so a
# case's leftovers do not leak into the next one's assertions.
clear_state() {
	local dir="${BENCH_STUB_STATE_DIR:-$STUBDIR}"
	for f in "$dir"/BENCH_STUB_BY_LABEL_* "$dir"/BENCH_STUB_PODS; do
		[ -f "$f" ] || continue
		rm -f "$f"
	done
}

# Source the file under test. PODMAN_SOCK and DOCKER_DEFAULT_SOCKET must be
# set in scope before sourcing leftovers.sh (run.sh exports them; the test
# mirrors that).
PATH="$STUBDIR/bin:$PATH"
PODMAN_SOCK="/tmp/test-podman.sock"
DOCKER_DEFAULT_SOCKET="/tmp/test-docker.sock"
export PATH PODMAN_SOCK DOCKER_DEFAULT_SOCKET
export BENCH_STUB_LOG="$STUBLOG" BENCH_STUB_STATE_DIR="$STUBDIR"
# shellcheck source=bench/leftovers.sh
. "$HERE/bench/leftovers.sh"

# bench_leftovers emits names one per line. The function's order is
# engine-sorted across containers, pods, networks; for assertions we sort
# the output into a deterministic form.
sorted_lines() { LC_ALL=C sort -u; }

# Case 1: podman-compose leaves four replicas, the pod and the network.
# The unrelated `alercom-api-1` (different label) and `alercom_default`
# (different label) must NOT be reported. The fixture rows store the
# FULL resource names the engine would carry (e.g.
# `s1947lb_podman_compose_scale_app_2`), with the label value being just
# the project name as the tool sets it (`-p <proj>`).
clear_state
load_table CONTAINERS io.podman.compose.project \
	s1947lb_podman_compose_scale s1947lb_podman_compose_scale_app_2 cid2 \
	s1947lb_podman_compose_scale s1947lb_podman_compose_scale_app_3 cid3 \
	s1947lb_podman_compose_scale s1947lb_podman_compose_scale_app_4 cid4 \
	s1947lb_podman_compose_scale s1947lb_podman_compose_scale_app_5 cid5 \
	alercom_api alercom_api_app_1 cid_a
load_table NETWORKS io.podman.compose.project \
	s1947lb_podman_compose_scale s1947lb_podman_compose_scale_default nid_d \
	alercom_api alercom_api_default nid_a
load_pods 'pod_s1947lb_podman_compose_scale'
got=$(bench_leftovers podman-compose s1947lb_podman_compose_scale | sorted_lines)
want=$(printf 's1947lb_podman_compose_scale_app_2\ns1947lb_podman_compose_scale_app_3\ns1947lb_podman_compose_scale_app_4\ns1947lb_podman_compose_scale_app_5\ns1947lb_podman_compose_scale_default\npod_s1947lb_podman_compose_scale' | sorted_lines)
check "podman-compose leftovers: app_2..5, pod, network" "$want" "$got"

# Purge: the stub logs every rm call. After purge, the foreign project's
# resources must NOT appear in the log (their label is different so the
# purge never asks for them).
: > "$STUBLOG"
bench_leftovers_purge podman-compose s1947lb_podman_compose_scale || true
if grep -E '^rm ' "$STUBLOG" | grep -F 'cid2' >/dev/null; then
	check "purge issues rm for container cid2" "yes" "yes"
else
	check "purge issues rm for container cid2" "yes" "no (log: $(grep -E '^rm ' "$STUBLOG" || echo none))"
fi
if grep -E '^rm ' "$STUBLOG" | grep -F 'cid_a' >/dev/null; then
	check "purge does NOT issue rm for foreign container cid_a (different label)" "no" "yes"
else
	check "purge does NOT issue rm for foreign container cid_a (different label)" "no" "no"
fi
if grep -E '^pod_rm ' "$STUBLOG" | grep -F 'pidx0' >/dev/null; then
	check "purge issues pod_rm for the pod" "yes" "yes"
else
	check "purge issues pod_rm for the pod" "yes" "no (log: $(grep -E '^pod_rm' "$STUBLOG" || echo none))"
fi
if grep -E '^network_rm ' "$STUBLOG" | grep -F 'nid_d' >/dev/null; then
	check "purge issues network_rm for the project's network" "yes" "yes"
else
	check "purge issues network_rm for the project's network" "yes" "no (log: $(grep -E '^network_rm' "$STUBLOG" || echo none))"
fi
if grep -E '^network_rm ' "$STUBLOG" | grep -F 'nid_a' >/dev/null; then
	check "purge does NOT issue network_rm for the foreign project's network" "no" "yes"
else
	check "purge does NOT issue network_rm for the foreign project's network" "no" "no"
fi

# Case 2: podup leaves containers and the network, never a pod. Same
# label key as the new code expects (`podup.project`).
clear_state
load_table CONTAINERS podup.project \
	s1947lb_podup_scale s1947lb_podup_scale-app-1 cid1 \
	s1947lb_podup_scale s1947lb_podup_scale-app-2 cid2 \
	s1947lb_podup_scale s1947lb_podup_scale-app-3 cid3
load_table NETWORKS podup.project \
	s1947lb_podup_scale s1947lb_podup_scale_default nid_d
load_pods
got=$(bench_leftovers podup s1947lb_podup_scale | sorted_lines)
want=$(printf 's1947lb_podup_scale-app-1\ns1947lb_podup_scale-app-2\ns1947lb_podup_scale-app-3\ns1947lb_podup_scale_default' | sorted_lines)
check "podup leftovers: app-1..3 and network" "$want" "$got"

# Case 3: foreign project. The label filter must exclude any container
# whose label value does not match the project. The foreign container
# `s1947lb_podup_scale_other-app-1` carries `alercom.project` and would
# have matched an old name-pattern regex like `^s1947lb_podup_scale-.+-[0-9]+$`;
# the new label-based query must NOT return it.
clear_state
load_table CONTAINERS alercom.project \
	s1947lb_podup_scale_other-app-1 cid_x \
	alercom-api-1 cid_a
load_table NETWORKS alercom.project \
	alercom_default alercom_default nid_a
load_pods
got=$(bench_leftovers podup s1947lb_podup_scale | sorted_lines)
check "foreign project (different label) is neither reported nor removed" "" "$got"

# Case 4: empty engine. None of the lists match; the function returns
# nothing.
clear_state
load_table CONTAINERS podup.project
load_table NETWORKS podup.project
load_pods
got=$(bench_leftovers podup s1947lb_podup_scale | sorted_lines)
check "empty engine: no leftovers reported" "" "$got"

# Case 5: docker-compose-docker, separator `-`, label
# `com.docker.compose.project`. The stub is at `docker`; the bench's
# `bench_engine_command` must prefix the call with `-H unix://...` so the
# docker CLI does not inherit the caller's DOCKER_HOST and hit another
# daemon.
clear_state
load_table CONTAINERS com.docker.compose.project \
	s1947lb_docker_compose_docker_scale s1947lb_docker_compose_docker_scale-app-1 cid1 \
	s1947lb_docker_compose_docker_scale s1947lb_docker_compose_docker_scale-app-2 cid2
load_table NETWORKS com.docker.compose.project \
	s1947lb_docker_compose_docker_scale s1947lb_docker_compose_docker_scale_default nid_d
load_pods
got=$(bench_leftovers docker-compose-docker s1947lb_docker_compose_docker_scale | sorted_lines)
want=$(printf 's1947lb_docker_compose_docker_scale-app-1\ns1947lb_docker_compose_docker_scale-app-2\ns1947lb_docker_compose_docker_scale_default' | sorted_lines)
check "docker-compose-docker leftovers: app-1..2 and network" "$want" "$got"

# The Docker host assertion: every docker invocation from
# bench_leftovers / bench_leftovers_purge for docker-compose-docker
# carries `-H unix://$DOCKER_DEFAULT_SOCKET`. The log file records every
# call, so the test can grep for the host flag. The leading `docker` arg
# is the program name and is not in `$*` (the stub sees only the args
# after argv[0]).
: > "$STUBLOG"
bench_leftovers docker-compose-docker s1947lb_docker_compose_docker_scale >/dev/null 2>&1 || true
bench_leftovers_purge docker-compose-docker s1947lb_docker_compose_docker_scale || true
if grep -E '^INVOKE -H unix:///tmp/test-docker\.sock ' "$STUBLOG" >/dev/null; then
	check "docker queries carry the explicit -H unix://\$DOCKER_DEFAULT_SOCKET" "present" "present"
else
	check "docker queries carry the explicit -H unix://\$DOCKER_DEFAULT_SOCKET" "present" "absent (log tail: $(tail -3 "$STUBLOG"))"
fi

# Case 6: podman-compose custom network (the `network-ipam` scenario
# creates `s1947lb_<tool>_network_ipam_app_net`, not `_default`). The
# previous round only checked `_default`, which would have missed this.
clear_state
load_table CONTAINERS io.podman.compose.project \
	s1947lb_podman_compose_network_ipam s1947lb_podman_compose_network_ipam_a_1 cid_a \
	s1947lb_podman_compose_network_ipam s1947lb_podman_compose_network_ipam_b_1 cid_b
load_table NETWORKS io.podman.compose.project \
	s1947lb_podman_compose_network_ipam s1947lb_podman_compose_network_ipam_app_net nid_app
load_pods 'pod_s1947lb_podman_compose_network_ipam'
got=$(bench_leftovers podman-compose s1947lb_podman_compose_network_ipam | sorted_lines)
want=$(printf 's1947lb_podman_compose_network_ipam_a_1\ns1947lb_podman_compose_network_ipam_b_1\ns1947lb_podman_compose_network_ipam_app_net\npod_s1947lb_podman_compose_network_ipam' | sorted_lines)
check "podman-compose custom network (network-ipam scenario) is reported" "$want" "$got"

# Case 7: bench_leftovers on an engine query failure must not return
# empty. A query command that fails must read as a hard failure (non-zero,
# message on stderr), not as "clean": an empty list there would silently
# pass for "the tool cleaned up".
clear_state
BENCH_STUB_FAIL_PS=1
export BENCH_STUB_FAIL_PS
got=$(bench_leftovers podup s1947lb_podup_scale 2>/dev/null)
rc=$?
check "engine query failure: bench_leftovers rc is non-zero (125)" "125" "$rc"
check "engine query failure: bench_leftovers stdout is empty (not 'clean')" "" "$got"

# The error message lands on stderr; the test reads it so a regression
# that silently swallows the error still shows.
err=$(bench_leftovers podup s1947lb_podup_scale 2>&1 >/dev/null)
unset BENCH_STUB_FAIL_PS
case "$err" in
	*"containers query failed"*|*"Error: stub"*) check "engine query failure: message on stderr" "yes" "yes" ;;
	*) check "engine query failure: message on stderr" "yes" "no: $err" ;;
esac

# Case 8: bench_check_leftovers returns BENCH_RC_LEFTOVERS when leftovers
# exist (and the engine query succeeded) and 0 when clean. The warning
# goes to stderr; we discard it.
clear_state
load_table CONTAINERS podup.project \
	s1947lb_podup_scale app-1 cid1
load_table NETWORKS podup.project \
	s1947lb_podup_scale s1947lb_podup_scale_default nid_d
load_pods
bench_check_leftovers podup s1947lb_podup_scale 2>/dev/null
check "bench_check_leftovers returns BENCH_RC_LEFTOVERS when leftovers exist" \
	"$BENCH_RC_LEFTOVERS" "$?"

load_table CONTAINERS podup.project
load_table NETWORKS podup.project
load_pods
bench_check_leftovers podup s1947lb_podup_scale 2>/dev/null
check "bench_check_leftovers returns 0 on a clean engine" "0" "$?"

# Case 9: bench_override_rc rewrites the rc field of a timeit output,
# keeping the first three fields intact.
clear_state
out="0.520 1436 0.110 0"
got=$(bench_override_rc "$out" "$BENCH_RC_LEFTOVERS")
check "bench_override_rc rewrites the rc field" \
	"0.520 1436 0.110 97" "$got"

# Case 10: bench_record_rc. The rc to record combines the tool's exit
# code with the leftover count: tool rc 0 + leftovers -> 97; tool rc
# non-zero + leftovers -> keep the tool's rc (it already failed);
# tool rc 0 + no leftovers -> 0; tool rc non-zero + no leftovers -> keep
# the tool's rc.
clear_state
check "bench_record_rc: tool rc 0 + leftovers -> 97" \
	"$BENCH_RC_LEFTOVERS" "$(bench_record_rc 0 'some-leftover')"
check "bench_record_rc: tool rc 125 + leftovers -> 125" \
	"125" "$(bench_record_rc 125 'some-leftover')"
check "bench_record_rc: tool rc 2 + leftovers -> 2" \
	"2" "$(bench_record_rc 2 'some-leftover')"
check "bench_record_rc: tool rc 0 + no leftovers -> 0" \
	"0" "$(bench_record_rc 0 '')"

# Sabotage tests: each one mutates the source file (or stub, for the
# format regression), re-runs a specific named case, and asserts the
# case fails. The source file is restored after each sabotage so the
# next normal test would still pass. Each sabotage targets exactly one
# named case from the list above.

# Sabotage 1: revert the pod/network .Name fix in bench/leftovers.sh
# (put .Names back). With .Names, the engine returns an rc=125 error
# (the stub rejects `{{.Names}}` to mimic the real CLI), and
# bench_leftovers fails instead of returning a list. The podman-compose
# leftovers case must then fail. Restored after, verified with diff -q.
cp "$HERE/bench/leftovers.sh" "$STUBDIR/leftovers.sh.bak"
sed -i "s|{{\.Name}}|{{.Names}}|g" "$HERE/bench/leftovers.sh"
# shellcheck source=bench/leftovers.sh
. "$HERE/bench/leftovers.sh"
clear_state
load_table CONTAINERS io.podman.compose.project \
	s1947lb_podman_compose_scale s1947lb_podman_compose_scale_app_2 cid2
load_table NETWORKS io.podman.compose.project \
	s1947lb_podman_compose_scale s1947lb_podman_compose_scale_default nid_d
load_pods 'pod_s1947lb_podman_compose_scale'
got=$(bench_leftovers podman-compose s1947lb_podman_compose_scale 2>/dev/null)
rc=$?
# Restore byte-for-byte from the backup.
cp "$STUBDIR/leftovers.sh.bak" "$HERE/bench/leftovers.sh"
# shellcheck source=bench/leftovers.sh
. "$HERE/bench/leftovers.sh"
# diff -q is silent when the two files are identical: that is the check.
diff -q "$STUBDIR/leftovers.sh.bak" "$HERE/bench/leftovers.sh"
restore_rc=$?
rm -f "$STUBDIR/leftovers.sh.bak"
if [ "$rc" = "125" ] && [ -z "$got" ]; then
	check "sabotage .Names makes podman-compose leftovers case fail (rc=125, no list)" "yes" "yes"
else
	check "sabotage .Names makes podman-compose leftovers case fail (rc=125, no list)" "yes" "no (rc=$rc, got=[$got])"
fi
check "sabotage 1 restore: diff -q silent on restored leftovers.sh" "0" "$restore_rc"

# Sabotage 2: drop the label filter from the engine query. bench_leftovers
# then returns every resource of the requested kind on the engine, not
# just the project's, so a foreign project whose NAME would have matched
# an old name-pattern regex would also be reported. The "foreign project
# (different label) is neither reported nor removed" case must then fail
# (it returns the foreign resources too).
#
# Patch: replace bench_leftovers with a sabotaged version that drops the
# --filter label=... part of the engine query. The stub returns every
# row in that case, which is what a real engine does without a filter.
bench_leftovers() { # tool project
	local tool="$1" proj="$2" cmd label_key out
	cmd=$(bench_engine_command "$tool") || return 2
	# shellcheck disable=SC2034 # label_key retained so the sabotage mirrors the production code (it never uses it because the filter is dropped)
	label_key=$(bench_project_label "$tool") || return 2
	# Sabotage: query without --filter label=...
	out=$($cmd ps -a --format '{{.Names}}') || {
		printf 'bench_leftovers: %s containers query failed\n' "$tool" >&2
		return 125
	}
	printf '%s\n' "$out"
	if [ "$tool" = podman-compose ]; then
		local pnames
		pnames=$(podman --url "unix://$PODMAN_SOCK" pod ls --format '{{.Name}}') || {
			printf 'bench_leftovers: podman-compose pods query failed\n' >&2
			return 125
		}
		grep -xF "pod_${proj}" <<<"$pnames" || true
	fi
	local nnames
	# Sabotage: query without --filter label=...
	nnames=$($cmd network ls --format '{{.Name}}') || {
		printf 'bench_leftovers: %s networks query failed\n' "$tool" >&2
		return 125
	}
	printf '%s\n' "$nnames"
}
clear_state
load_table CONTAINERS alercom.project \
	s1947lb_podup_scale_other-app-1 cid_x \
	alercom-api-1 cid_a
load_table NETWORKS alercom.project \
	alercom_default alercom_default nid_a
load_pods
got=$(bench_leftovers podup s1947lb_podup_scale 2>/dev/null | sorted_lines)
# Restore: re-source the file.
# shellcheck source=bench/leftovers.sh
. "$HERE/bench/leftovers.sh"
# Foreign resources have alercom.project label, not podup.project; with
# the filter dropped, they leak in.
if [ -n "$got" ]; then
	check "sabotage label filter makes foreign-project case fail (foreign leaked in)" "yes" "yes"
else
	check "sabotage label filter makes foreign-project case fail (foreign leaked in)" "yes" "no (empty)"
fi

# Sanity: the foreign-project case passes again after restore.
clear_state
load_table CONTAINERS alercom.project \
	s1947lb_podup_scale_other-app-1 cid_x \
	alercom-api-1 cid_a
load_table NETWORKS alercom.project \
	alercom_default alercom_default nid_a
load_pods
got=$(bench_leftovers podup s1947lb_podup_scale 2>/dev/null | sorted_lines)
check "sabotage 2 restore: foreign-project case passes again" "" "$got"

# Sabotage 3: replace bench_record_rc with one that always returns 97,
# ignoring the tool's own rc. The "tool rc 125 + leftovers -> 125" case
# must then fail.
bench_record_rc() { printf '%s\n' "$BENCH_RC_LEFTOVERS"; }
check "sabotage rc-preserve: bench_record_rc now always returns 97 (case fails)" \
	"$BENCH_RC_LEFTOVERS" "$(bench_record_rc 125 'some-leftover')"
# Restore: re-source.
# shellcheck source=bench/leftovers.sh
. "$HERE/bench/leftovers.sh"
check "sabotage 3 restore: bench_record_rc keeps tool rc 125" "125" "$(bench_record_rc 125 'some-leftover')"

echo
echo "$pass passed, $fail failed"
printf 'DONE %s %d %d\n' "${BASH_SOURCE[0]##*/}" "$pass" "$fail"
[ "$fail" -eq 0 ]
