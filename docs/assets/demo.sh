#!/usr/bin/env bash
# Records the README demo: `podup up -d`, `podup ps`, `podup down` on a
# two-service stack. Run it under a recorder, for example:
#   asciinema rec --window-size 110x16 -c docs/assets/demo.sh demo.cast
#   agg --theme monokai --font-size 16 demo.cast docs/assets/podup-demo.gif
set -euo pipefail

PODUP=${PODUP:-podup}
base=$(mktemp -d)
work="$base/podup-demo"
mkdir "$work"
trap 'cd /; "$PODUP" -f "$work/compose.yaml" down >/dev/null 2>&1 || true; rm -rf "$base"' EXIT
cd "$work"
cat > compose.yaml <<'YAML'
services:
  web:
    image: docker.io/library/nginx:alpine
    ports:
      - "127.0.0.1:8080:80"
  cache:
    image: docker.io/library/redis:alpine
YAML
# Pull first so the recording shows the stack starting, not a download.
podman pull -q docker.io/library/nginx:alpine docker.io/library/redis:alpine >/dev/null

run() {
	printf '\033[1;32m$\033[0m '
	for ((i = 0; i < ${#1}; i++)); do
		printf '%s' "${1:i:1}"
		sleep 0.04
	done
	sleep 0.4
	printf '\n'
	eval "${1/podup/$PODUP}"
	sleep 1.6
	printf '\n'
}

clear
run "podup up -d"
run "podup ps"
run "podup down"
