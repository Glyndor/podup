#!/usr/bin/env bash
# Fair compose-tool benchmark runner.
#
# Drives each tool through the same scenario suite, the same number of times,
# on the same machine, against digest-pinned, pre-pulled images. Each timed run
# goes through bench/timeit, so every row records wall-clock, peak resident
# memory and CPU time of the orchestrator process; the statistics (median / p95 /
# stdev) are computed by aggregate.py, never here.
#
# Fairness is the whole point: identical compose inputs, identical lifecycle,
# warm-up iterations discarded, every scenario reported, the same op flags for
# every tool. Same-engine tools (podup, podman-compose) drive Podman and are a
# pure tool comparison; docker-compose is run TWICE in a single invocation,
# once against Podman (pure tool, what the README publishes) and once against
# Docker (whole-stack, what a Docker user actually runs). Each variant is its
# own row in raw.csv, so one run can hold both comparisons.
#
# Note on the memory/CPU columns: they are the resource use of the tool process
# and the processes it directly spawns and waits on (getrusage). podup is a thin
# client to the long-running Podman service, so engine-side work is not charged
# to it; podman-compose shells out to the `podman` binary per call, whose work it
# waits on and is charged for. The columns therefore measure client-side cost per
# command, what running the tool costs on your machine, not engine work.
#
# Usage: bench/run.sh [--iters N] [--warmup W] [--cores CPUSET] [--engines LIST]
#                     [--allow-dynamic] [--smoke]
set -u

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCEN_DIR="$HERE/scenarios"
OUT_DIR="$HERE/results"
RAW="$OUT_DIR/raw.csv"
TIMEIT_DIR="$HERE/timeit"
TIMEIT="$TIMEIT_DIR/target/release/timeit"

ITERS=12
WARMUP=2
CORES=""
SMOKE=0
ENGINES="podman,docker"
ALLOW_DYNAMIC=0
PODUP_BIN="${PODUP_BIN:-podup}"

while [ $# -gt 0 ]; do
	case "$1" in
		--iters) ITERS="$2"; shift 2 ;;
		--warmup) WARMUP="$2"; shift 2 ;;
		--cores) CORES="$2"; shift 2 ;;
		--engines) ENGINES="$2"; shift 2 ;;
		--allow-dynamic) ALLOW_DYNAMIC=1; shift ;;
		--smoke) SMOKE=1; ITERS=1; WARMUP=0; shift ;;
		*) echo "unknown arg: $1" >&2; exit 2 ;;
	esac
done

mkdir -p "$OUT_DIR"

# The timer is on the measured path, so build it before the first scenario
# rather than discovering it is missing after half an hour of empty rows.
if [ ! -x "$TIMEIT" ]; then
	echo ">>> building the timer (bench/timeit)"
	cargo build --release --manifest-path "$TIMEIT_DIR/Cargo.toml" >/dev/null || {
		echo "bench: could not build $TIMEIT_DIR" >&2
		exit 2
	}
fi

# Scenario list and the op-group each one measures.
#   updown  : time `up -d` and `down -v`
#   scale   : like updown but `up -d --scale app=5`
#   reup    : time a warm second `up -d`
#   running : bring up untimed, then time `ps`, `logs`, `exec -T`, `restart`
#   build   : time `build --no-cache`
SCENARIOS=(single multi-healthcheck deep-chain wide-level scale network-ipam volume-heavy secrets warm-restart many-services running-ops wide-running-ops config-heavy build)
declare -A OP=(
	[single]=updown [multi-healthcheck]=updown [scale]=scale
	[network-ipam]=updown [volume-heavy]=updown [secrets]=updown [warm-restart]=reup
	[many-services]=updown [running-ops]=running [build]=build
	[config-heavy]=parse [wide-running-ops]=running
	[deep-chain]=updown [wide-level]=updown
)

# Every scenario must have an op, or the run dies partway through with an
# unbound-variable error under `set -u`. deep-chain and wide-level were added to
# SCENARIOS in #1123 and never added here, so the suite has been unable to
# complete since; nobody noticed because those two were only ever run by hand,
# one at a time, to measure the scheduler change that introduced them.
for _s in "${SCENARIOS[@]}"; do
	[ -n "${OP[$_s]:-}" ] || { echo "bench: scenario '$_s' has no entry in OP" >&2; exit 2; }
done
if [ "$SMOKE" -eq 1 ]; then SCENARIOS=(single running-ops); fi

# Same-engine tools (podup, podman-compose) always run. docker-compose is split
# into two variants (one per engine), each gated on the engine answering at
# its own DOCKER_HOST and (for the Docker one) the docker CLI being installed
# to pre-pull the images. The caller's DOCKER_HOST is irrelevant: every variant
# sets its own, so the run no longer depends on the shell it was launched from.
TOOLS=(podup podman-compose)
WANT_PODMAN=0; WANT_DOCKER=0
case ",$ENGINES," in
	*,podman,*) WANT_PODMAN=1 ;;
esac
case ",$ENGINES," in
	*,docker,*) WANT_DOCKER=1 ;;
esac
PODMAN_SOCK="${PODMAN_SOCKET:-$XDG_RUNTIME_DIR/podman/podman.sock}"

# shellcheck source=bench/engine.sh
. "$HERE/engine.sh"
# shellcheck source=bench/env.sh
. "$HERE/env.sh"
MEASURE_DC_PODMAN=0
MEASURE_DC_DOCKER=0

if ! command -v docker-compose >/dev/null 2>&1; then
	[ "$WANT_PODMAN" -eq 1 ] && echo "note: docker-compose not installed; docker-compose-podman NOT measured."
	[ "$WANT_DOCKER" -eq 1 ] && echo "note: docker-compose not installed; docker-compose-docker NOT measured."
else
	if [ "$WANT_PODMAN" -eq 1 ]; then
		if engine="$(DOCKER_HOST="unix://$PODMAN_SOCK" compose_engine)" && [ "$engine" = podman ]; then
			MEASURE_DC_PODMAN=1
			echo "note: docker-compose-podman driving Podman (unix://$PODMAN_SOCK); measured as a SAME-ENGINE (pure tool) run."
		else
			echo "note: docker-compose found but DOCKER_HOST=unix://$PODMAN_SOCK did not answer as Podman (${engine:-unreachable}); docker-compose-podman NOT measured."
		fi
	fi
	if [ "$WANT_DOCKER" -eq 1 ]; then
		if engine="$(DOCKER_HOST="unix://$DOCKER_DEFAULT_SOCKET" compose_engine)" && [ "$engine" = docker ]; then
			if command -v docker >/dev/null 2>&1; then
				MEASURE_DC_DOCKER=1
				echo "note: docker-compose-docker driving Docker (unix://$DOCKER_DEFAULT_SOCKET); measured as a CROSS-ENGINE (whole-stack) run."
			else
				echo "note: docker-compose found Docker at the default socket but the docker CLI is missing, so its images cannot be pre-pulled; docker-compose-docker NOT measured."
			fi
		else
			echo "note: docker-compose found but DOCKER_HOST=unix://$DOCKER_DEFAULT_SOCKET did not answer as Docker (${engine:-unreachable}); docker-compose-docker NOT measured."
		fi
	fi
fi

[ "$MEASURE_DC_PODMAN" -eq 1 ] && TOOLS+=(docker-compose-podman)
[ "$MEASURE_DC_DOCKER" -eq 1 ] && TOOLS+=(docker-compose-docker)

run() { # tool, compose-file, project, op-args...
	local tool="$1" file="$2" proj="$3"; shift 3
	local pre=(); [ -n "$CORES" ] && pre=(taskset -c "$CORES")
	# `file` may name several compose files separated by spaces, so a scenario can
	# exercise the base+override merge every real project has. One name yields one
	# -f, exactly as before.
	local fargs=(); local f; for f in $file; do fargs+=(-f "$f"); done
	case "$tool" in
		podup)                 "${pre[@]}" "$PODUP_BIN" "${fargs[@]}" -p "$proj" "$@" ;;
		podman-compose)        "${pre[@]}" podman-compose "${fargs[@]}" -p "$proj" "$@" ;;
		docker-compose-podman) "${pre[@]}" env "DOCKER_HOST=unix://$PODMAN_SOCK" docker-compose "${fargs[@]}" -p "$proj" "$@" ;;
		docker-compose-docker) "${pre[@]}" env "DOCKER_HOST=unix://$DOCKER_DEFAULT_SOCKET" docker-compose "${fargs[@]}" -p "$proj" "$@" ;;
	esac
}

# Builds the real external command for a tool (the timer execs it, so it cannot
# be a shell function) and echoes "wall_s max_rss_kb cpu_s rc".
timed() { # tool, compose-file, project, op-args...
	local tool="$1" file="$2" proj="$3"; shift 3
	local cmd=(); [ -n "$CORES" ] && cmd=(taskset -c "$CORES")
	local fargs=(); local f; for f in $file; do fargs+=(-f "$f"); done
	case "$tool" in
		podup)                 cmd+=("$PODUP_BIN" "${fargs[@]}" -p "$proj" "$@") ;;
		podman-compose)        cmd+=(podman-compose "${fargs[@]}" -p "$proj" "$@") ;;
		docker-compose-podman) cmd+=(env "DOCKER_HOST=unix://$PODMAN_SOCK" docker-compose "${fargs[@]}" -p "$proj" "$@") ;;
		docker-compose-docker) cmd+=(env "DOCKER_HOST=unix://$DOCKER_DEFAULT_SOCKET" docker-compose "${fargs[@]}" -p "$proj" "$@") ;;
	esac
	LC_ALL=C "$TIMEIT" "${cmd[@]}"
}

teardown() { run "$1" "$2" "$3" down -v >/dev/null 2>&1; }

# Pre-pull the digest-pinned bases so image download is never on the timed path.
#
# LC_ALL=C on the sort: UTF-8 collations ignore punctuation at the primary
# level, so `sort -u` treats two image references that differ only in a hyphen
# as equal and drops one. That image is then pulled during the run instead of
# before it, and the benchmark times a network download rather than a start --
# silently, as a number that is simply larger.
echo ">>> pre-pulling pinned images"
grep -rhoE 'docker\.io/[^ "]+@sha256:[a-f0-9]+' "$SCEN_DIR" | LC_ALL=C sort -u | while read -r img; do
	podman pull -q "$img" >/dev/null 2>&1 || echo "  warning: could not pre-pull $img" >&2
	# Docker keeps its own image store, so a Docker run needs its own copy or
	# the first timed `up` measures the download. Only relevant when the Docker
	# variant is actually being measured in this run.
	if [ "$MEASURE_DC_DOCKER" -eq 1 ]; then
		docker pull -q "$img" >/dev/null 2>&1 || echo "  warning: could not pre-pull $img into Docker" >&2
	fi
done

# Record the environment next to raw.csv. aggregate.py embeds this file in
# report.md, so a reader can tell which podup, which Podman, which kernel and
# CPU governor produced the numbers below. Written AFTER pre-pull so the
# engine-state counters (containers, images, dangling) reflect what the run
# will actually exercise and not what was there when the script started.
write_env "$OUT_DIR/env.txt"
echo ">>> environment:"
sed 's/^/    /' "$OUT_DIR/env.txt"

# Refuse to publish numbers measured against the wrong artifact. The published
# memory budget is for the static release asset; a glibc `cargo build
# --release` on the same source tree reports about twice the RSS, which the
# budget gate then rejects anyway. Catching it here keeps the run short and
# the message in the operator's shell rather than buried in CI output.
PODUP_LINKAGE="$(awk -F': ' '/^podup_linkage: /{print $2; exit}' "$OUT_DIR/env.txt")"
if [ "$PODUP_LINKAGE" = "dynamically linked" ]; then
	if [ "$ALLOW_DYNAMIC" -eq 1 ]; then
		echo "warning: podup is dynamically linked; numbers will not match the static release asset."
	else
		echo "bench: podup is dynamically linked; the published numbers measure the static release asset, not this build. Pass --allow-dynamic to override." >&2
		exit 2
	fi
fi

# Soft warnings: each one makes a published number noisier, but none of them
# changes the fairness of the comparison, so they stay on stderr and the run
# continues. Reader-facing: see report.md's Environment block for the values.
PODMAN_DANGLING="$(awk -F': ' '/^podman_dangling_images: /{print $2; exit}' "$OUT_DIR/env.txt")"
RUNNING_VMS="$(awk -F': ' '/^running_vms: /{print $2; exit}' "$OUT_DIR/env.txt")"
GOVERNOR="$(awk -F': ' '/^governor: /{print $2; exit}' "$OUT_DIR/env.txt")"
if [ "${PODMAN_DANGLING:-0}" -gt 0 ] 2>/dev/null; then
	echo "warning: $PODMAN_DANGLING dangling podman image(s); leftover images/networks slow podman listing calls, which podman-compose makes on every command; clean them before a published run." >&2
fi
if [ "${RUNNING_VMS:-0}" -gt 0 ] 2>/dev/null; then
	echo "warning: $RUNNING_VMS running VM(s) sharing the same cores; numbers will be noisier." >&2
fi
if [ -n "$GOVERNOR" ] && [ "$GOVERNOR" != performance ] && [ "$GOVERNOR" != unknown ]; then
	echo "warning: cpu governor is '$GOVERNOR', not 'performance'; numbers will be noisier." >&2
fi

echo "tool,scenario,op,iter,phase,seconds,max_rss_kb,cpu_s,rc" > "$RAW"

for tool in "${TOOLS[@]}"; do
	for scen in "${SCENARIOS[@]}"; do
		file="$SCEN_DIR/$scen/compose.yaml"
		# A scenario may ship an override alongside its base file; merging the two
		# is what real projects do and what no single-file scenario can measure.
		# Passed explicitly rather than relying on auto-discovery, since the tools
		# disagree about that and this must compare the same work.
		[ -f "$SCEN_DIR/$scen/compose.override.yaml" ] &&
			file="$file $SCEN_DIR/$scen/compose.override.yaml"
		op="${OP[$scen]}"
		proj="bench_${tool//-/_}_${scen//-/_}"
		echo ">>> $tool / $scen (op=$op)"
		teardown "$tool" "$file" "$proj"
		for ((i=0; i<ITERS; i++)); do
			phase="measured"; [ "$i" -lt "$WARMUP" ] && phase="warmup"
			row() { echo "$tool,$scen,$1,$i,$phase,${2// /,}" >> "$RAW"; }
			case "$op" in
				updown)
					row up   "$(timed "$tool" "$file" "$proj" up -d)"
					row down "$(timed "$tool" "$file" "$proj" down -v)"
					;;
				scale)
					row up   "$(timed "$tool" "$file" "$proj" up -d --scale app=5)"
					row down "$(timed "$tool" "$file" "$proj" down -v)"
					;;
				reup)
					run "$tool" "$file" "$proj" up -d >/dev/null 2>&1
					row reup "$(timed "$tool" "$file" "$proj" up -d)"
					teardown "$tool" "$file" "$proj"
					;;
				running)
					run "$tool" "$file" "$proj" up -d >/dev/null 2>&1
					row ps      "$(timed "$tool" "$file" "$proj" ps)"
					row logs    "$(timed "$tool" "$file" "$proj" logs app)"
					row exec    "$(timed "$tool" "$file" "$proj" exec -T app true)"
					row restart "$(timed "$tool" "$file" "$proj" restart app)"
					teardown "$tool" "$file" "$proj"
					;;
				parse)
					# The only op with no engine on the other side: read, interpolate,
					# merge and re-render. No containers means no daemon variance, so
					# this is the least noisy number the suite produces, and it is
					# what CI runs most, to validate a file before deploying it.
					row config "$(timed "$tool" "$file" "$proj" config)"
					;;
				build)
					row build "$(timed "$tool" "$file" "$proj" build --no-cache)"
					if [ "$tool" = docker-compose-docker ]; then
						docker rmi -f "podup-bench-build:latest" >/dev/null 2>&1
					else
						podman rmi -f "podup-bench-build:latest" >/dev/null 2>&1
					fi
					;;
			esac
		done
		teardown "$tool" "$file" "$proj"
	done
done

echo "raw rows: $(( $(wc -l < "$RAW") - 1 )) -> $RAW"
