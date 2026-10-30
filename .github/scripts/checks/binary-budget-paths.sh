#!/usr/bin/env bash
# Exit 0 only for a non-empty diff confined to these reviewed test modules.
# Each path is declared by #[cfg(test)] followed by #[path = "..."] in its
# production sibling: create.rs, transport.rs, sync.rs, mod.rs, events.rs.
# No suffix or directory wildcard is evidence that a Rust file is test-only.
# Parent edits, new modules, manifests, build helpers and all unknown paths build.
set -uo pipefail
export LC_ALL=C

build() {
	echo "release build required: $*" >&2
	exit 1
}

[ "$#" -eq 1 ] || build "usage: binary-budget-paths.sh CHANGED_FILES"
[ -n "$1" ] || build "empty diff"
while IFS= read -r path; do
	case "$path" in
		internal/backends/engine/objects/secrets/create_tests.rs | \
		internal/backends/engine/host_files/copy/pack/transport_tests.rs | \
		internal/backends/engine/host_files/watch/sync_filter_tests.rs | \
		internal/backends/engine/observe/stats/stats_tests.rs | \
		internal/backends/engine/host_files/watch/filtering/events_tests.rs) ;;
		*) build "$path" ;;
	esac
done <<< "$1"
echo "only reviewed cfg(test) modules changed"
