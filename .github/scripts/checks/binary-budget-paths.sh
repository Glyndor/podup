#!/usr/bin/env bash
# Exit 0 only for a non-empty diff confined to these reviewed test modules.
# Each module is declared by #[cfg(test)] followed by #[path = "..."] in its
# production sibling (create.rs, transport.rs, sync.rs, mod.rs, events.rs); the
# four that are folders list the files of the folder one by one.
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
		internal/backends/engine/objects/secrets/create_tests/environment.rs | \
		internal/backends/engine/objects/secrets/create_tests/mod.rs | \
		internal/backends/engine/objects/secrets/create_tests/mode.rs | \
		internal/backends/engine/objects/secrets/create_tests/replace.rs | \
		internal/backends/engine/objects/secrets/create_tests/sizes.rs | \
		internal/backends/engine/host_files/copy/pack/transport_tests/closing.rs | \
		internal/backends/engine/host_files/copy/pack/transport_tests/mod.rs | \
		internal/backends/engine/host_files/copy/pack/transport_tests/parity.rs | \
		internal/backends/engine/host_files/copy/pack/transport_tests/recording.rs | \
		internal/backends/engine/host_files/copy/pack/transport_tests/refusals.rs | \
		internal/backends/engine/host_files/watch/sync_filter_tests/filter.rs | \
		internal/backends/engine/host_files/watch/sync_filter_tests/mod.rs | \
		internal/backends/engine/host_files/watch/sync_filter_tests/permitted.rs | \
		internal/backends/engine/host_files/watch/sync_filter_tests/pipe.rs | \
		internal/backends/engine/host_files/watch/sync_filter_tests/swaps.rs | \
		internal/backends/engine/host_files/watch/sync_filter_tests/unsyncable.rs | \
		internal/backends/engine/observe/stats/stats_tests/bytes.rs | \
		internal/backends/engine/observe/stats/stats_tests/frame.rs | \
		internal/backends/engine/observe/stats/stats_tests/header.rs | \
		internal/backends/engine/observe/stats/stats_tests/json.rs | \
		internal/backends/engine/observe/stats/stats_tests/mod.rs | \
		internal/backends/engine/observe/stats/stats_tests/model.rs | \
		internal/backends/engine/observe/stats/stats_tests/network.rs | \
		internal/backends/engine/observe/stats/stats_tests/query.rs | \
		internal/backends/engine/observe/stats/stats_tests/row.rs | \
		internal/backends/engine/observe/stats/stats_tests/stream.rs | \
		internal/backends/engine/observe/stats/stats_tests/style.rs | \
		internal/backends/engine/host_files/watch/filtering/events_tests.rs) ;;
		*) build "$path" ;;
	esac
done <<< "$1"
echo "only reviewed cfg(test) modules changed"
