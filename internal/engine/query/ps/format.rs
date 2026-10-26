//! Cell renderers for the `ps` table and the URL builder for the libpod list
//! request, plus the shared `now_unix` clock that both `ps` and `images` read
//! (so a row's CREATED age and an image's CREATED age are clocked against the
//! same instant).
//!
//! Pure: every cell renders from what libpod already sent, the URL is built
//! from the project label, and `now_unix` returns zero on a clock that has
//! not seen the epoch. Each is unit-tested without a live Podman socket.

use crate::libpod::types::container::ContainerListEntry;
use crate::units::{format_bytes, format_duration, DurationFormat, SizeFormat};

use super::super::inspect::humanize_age;

/// Human-readable status for `ps`. Podman's libpod list endpoint leaves
/// `Status` empty and reports the machine state in `State`, so fall back to it
/// rather than rendering a blank column.
pub(super) fn display_status(c: &ContainerListEntry) -> &str {
	if c.status.is_empty() {
		&c.state
	} else {
		&c.status
	}
}

/// How `ps` renders a span: the shared default, three components.
const SPAN_FORMAT: DurationFormat = DurationFormat::default_parts();

/// Table STATUS cell, as of `now` (Unix seconds).
///
/// A running container says how long it has been up, which is what both
/// reference tools do and what podup did not: `podman ps` renders `Up 13 hours
/// (healthy)` and `docker compose ps` renders `Up 2 minutes`, while podup
/// rendered the bare word `running`. The state alone answers "is it on"; the
/// span answers "did it just restart", which is the question someone runs `ps`
/// to settle.
///
/// The health suffix only appears when the container has a healthcheck:
/// libpod leaves `Status` empty otherwise, measured on Podman 5.7.0, so there is
/// nothing to append rather than an unknown to invent.
///
/// Podman's list endpoint reports an exited container with a bare `exited` state
/// and no code, which is indistinguishable from a clean exit; surface the exit
/// code the way `docker compose ps` does (`Exited (0)` / `Exited (7)`).
///
/// `now` is a parameter rather than a clock read so the rendering is pure and
/// its tests are deterministic.
pub(super) fn table_status(c: &ContainerListEntry, now: i64) -> String {
	let status = display_status(c);
	let exited = c.state.eq_ignore_ascii_case("exited") || c.state.eq_ignore_ascii_case("dead");
	// Only synthesize when the status text doesn't already carry the code, so a
	// richer Docker-style `Exited (7) 4 seconds ago` is left untouched.
	if exited && !status.contains("Exited (") {
		return format!("Exited ({})", c.exit_code.unwrap_or(0));
	}
	if !c.state.eq_ignore_ascii_case("running") {
		return status.to_string();
	}
	let Some(uptime) = span_since(c.started_at, now) else {
		return status.to_string();
	};
	// `health_from_status` answers with an empty string when the container has
	// no healthcheck, which is the same case as libpod sending an empty
	// `Status`, so nothing to append rather than an unknown to invent.
	match super::filters::health_from_status(status) {
		"" => format!("Up {uptime}"),
		health => format!("Up {uptime} ({health})"),
	}
}

/// The libpod list request `ps` makes, as a URL.
///
/// Pure so the query it builds can be asserted without a server. The `size`
/// parameter is the one worth pinning: `size=true` is not a bigger payload, it
/// is work: libpod walks each container's writable layer to answer, measured
/// at 21 ms against 109 ms over 59 containers on Podman 5.7.0, so asking for it
/// unconditionally would make every `ps` pay for a column most readers do not
/// look at. `docker ps -s` is opt-in for the same reason.
///
/// Extracted after a mutation that hard-coded `size=true` survived every test:
/// the string was built inside an async method that needs a live socket, so
/// nothing could reach it.
pub(super) fn containers_path(project: &str, all: bool, size: bool) -> String {
	use crate::libpod::{urlencoded, API_PREFIX};
	let label = format!("podup.project={project}");
	let filters = serde_json::json!({ "label": [label] });
	format!(
		"{API_PREFIX}/containers/json?all={all}&size={size}&filters={}",
		urlencoded(&filters.to_string()),
	)
}

/// How `ps` renders a size: decimal units at three significant digits.
///
/// The same shape `images` uses, and for the same reason: this column is read
/// against `podman ps -s`, which prints `143kB (virtual 225MB)`. Measured
/// against it rather than assumed.
const SIZE_FORMAT: SizeFormat = SizeFormat::decimal().with_significant(3);

/// Table SIZE cell: the writable layer, then the image behind it.
///
/// `143kB (virtual 225MB)`, matching `podman ps -s` and `docker ps -s`.
/// **`virtual` is the image's own size, not the sum**, verified on three
/// containers whose two readings differ at three significant digits, because on
/// a container with a small writable layer the two are indistinguishable and a
/// wrong choice would never show.
///
/// Empty when the server sent no size, which is what it does unless the request
/// asked. A blank cell says podup did not ask; `0B` would claim it did.
pub(super) fn table_size(c: &ContainerListEntry) -> String {
	let Some(size) = &c.size else {
		return String::new();
	};
	format!(
		"{} (virtual {})",
		format_bytes(size.rw, &SIZE_FORMAT),
		format_bytes(size.root_fs, &SIZE_FORMAT)
	)
}

/// Table CREATED cell, as of `now`: how long ago the container was created.
///
/// Empty when libpod sent no timestamp or one this cannot parse. A blank cell
/// says podup could not tell; a cell holding a plausible wrong age does not, and
/// the wrong age is the one a reader acts on.
pub(super) fn table_created(c: &ContainerListEntry, now: i64) -> String {
	let Some(secs) = crate::timestamp::parse_rfc3339(&c.created) else {
		return String::new();
	};
	if secs <= 0 {
		return String::new();
	}
	humanize_age(now.saturating_sub(secs).max(0))
}

/// Table SERVICE cell: the container's service name.
///
/// Read from the `podup.service` label libpod carries on every project
/// container (set at `up`, see `internal/engine/workload/container/inputs.rs`). Empty
/// when libpod sent no label; the row already says "this is a project
/// container" by carrying the project label, so a blank service means
/// "unknown", which is what an empty cell says elsewhere in this table.
pub(super) fn table_service(c: &ContainerListEntry) -> String {
	c.labels.get("podup.service").cloned().unwrap_or_default()
}

/// Render the span from `then` to `now`, or `None` when there is nothing to
/// render.
///
/// A zero `then` means the field was absent, not that the instant was the epoch.
/// A `then` in the future is clock skew between this process and the server,
/// clamped to zero rather than rendered as a negative age, since `Up 0s` on a
/// container that just started is right and `Up -3s` is never right.
pub(super) fn span_since(then: i64, now: i64) -> Option<String> {
	if then <= 0 {
		return None;
	}
	let elapsed = now.saturating_sub(then).max(0);
	Some(format_duration(
		std::time::Duration::from_secs(elapsed as u64),
		&SPAN_FORMAT,
	))
}

/// The wall clock as Unix seconds, for the cells that render an age.
///
/// Before the epoch is not a state this can reach on any host that boots, so a
/// failure floors at zero and every cell renders blank rather than the call
/// site handling an error it cannot act on.
///
/// `pub(in crate::engine)` because `images.rs` (in `crate::engine::query`)
/// reads from the same instant so its CREATED column and `ps`'s CREATED column
/// never disagree. `pub(super)` from `format.rs` already reaches that
/// scope; the explicit `pub(in ...)` keeps the reach identical to the
/// pre-split module placement.
pub(in crate::engine) fn now_unix() -> i64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|d| d.as_secs() as i64)
		.unwrap_or(0)
}

/// The container's display name (leading slash stripped). Used by both the
/// table rendering and the JSON row builder, so held here at `pub(super)`
/// reach (`crate::engine::query::ps::*`) so either caller can take it
/// without routing through the other.
pub(super) fn name_of(c: &ContainerListEntry) -> String {
	c.names.join(", ").trim_start_matches('/').to_string()
}
