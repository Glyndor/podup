# shellcheck shell=bash
# Which engine docker-compose will drive, read from the engine itself.
#
# Sourced by run.sh. It used to decide with `docker info`, which answers a
# different question: whether a `docker` CLI exists and reaches something. The
# CLI honours DOCKER_HOST, so on a host with Docker installed and DOCKER_HOST
# pointed at the Podman socket, `docker info` succeeded against Podman and the
# run was labelled a Docker (cross-engine) comparison. The same-engine
# comparison, the one docs/benchmarks.md publishes, could no longer be selected
# at all once Docker was installed.
#
# docker-compose talks to DOCKER_HOST when it is set and to the default Docker
# socket otherwise, so that is the endpoint probed here. `/version` names the
# server: Podman lists a component called "Podman Engine", Docker does not.

DOCKER_DEFAULT_SOCKET="${DOCKER_DEFAULT_SOCKET:-/var/run/docker.sock}"

# Prints "podman" or "docker" and returns 0, or prints why the engine is
# unknown and returns 1.
compose_engine() {
	local host="${DOCKER_HOST:-}" args=() url body
	if [ -z "$host" ]; then
		# A docker context other than the default would send compose somewhere
		# this probe does not look.
		if [ -n "${DOCKER_CONTEXT:-}" ] && [ "$DOCKER_CONTEXT" != default ]; then
			echo "DOCKER_CONTEXT=$DOCKER_CONTEXT is set; set DOCKER_HOST instead so the engine can be identified"
			return 1
		fi
		host="unix://$DOCKER_DEFAULT_SOCKET"
	fi
	case "$host" in
		unix://*) args=(--unix-socket "${host#unix://}"); url="http://engine/version" ;;
		tcp://*) url="http://${host#tcp://}/version" ;;
		*) echo "cannot probe DOCKER_HOST=$host (only unix:// and tcp://)"; return 1 ;;
	esac
	if ! body="$(curl -fsS --max-time 5 "${args[@]}" "$url" 2>/dev/null)"; then
		echo "no engine answered at $host"
		return 1
	fi
	case "$body" in
		*'"Podman Engine"'*) echo podman ;;
		*'"ApiVersion"'*) echo docker ;;
		*) echo "the server at $host is neither Podman nor Docker"; return 1 ;;
	esac
}
