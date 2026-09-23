//! What the buildah stream says, read line by line.
//!
//! The libpod build endpoint answers with buildah's own output, one JSON line
//! per text line. Three shapes matter to the board: `STEP n/m: <instruction>`,
//! which moves the image row's verb; the bare 64-hex image id buildah prints
//! after `Successfully tagged`, which a script reading `build` from a pipe
//! wants on stdout; and the failure path, where the stream a terminal folded
//! away is replayed once so the reason is on screen (#1681).
//!
//! On the failure path a single extra call is paid: `GET /libpod/info`, to
//! read `host.cgroupManager`. When the value is `systemd`, the build error is
//! extended with one hint line about `cgroup_manager = "cgroupfs"` in
//! `~/.config/containers/containers.conf` (#1778). The call runs only on a
//! build failure; a successful build never issues it. If the call itself
//! fails, the original error message reaches the caller unchanged.
use std::io::IsTerminal;

use serde::Deserialize;

use crate::engine::Engine;
use crate::error::ComposeError;
use crate::libpod::API_PREFIX;

impl Engine {
	/// Close the row as `Failed`, then on a terminal replay the full stream
	/// as scrollback so the failure reason is on screen. In a pipe every line
	/// has already been written by `note_for` and no replay is needed.
	///
	/// On the way out, fetch `host.cgroupManager` over the same socket the
	/// build used. If it is `systemd`, append one hint line naming
	/// `cgroup_manager = "cgroupfs"` and the file the key goes in, so a
	/// reader running the Linux build inside a `podman-machine` WSL distro
	/// sees the setting instead of an opaque build failure (#1778). On any
	/// other value (or on an info call that fails for any reason), the
	/// original message reaches the caller unchanged: the hint is a hint,
	/// not a diagnosis, and a swallowed runtime error would be worse than
	/// no hint at all.
	pub(super) async fn fail_build(
		&self,
		tag: &str,
		err: String,
		capture: Vec<String>,
		quiet: bool,
	) -> ComposeError {
		if !quiet {
			crate::ui::progress_line("Image", tag, "Failed");
		}
		if !quiet && std::io::stderr().is_terminal() {
			use std::io::Write;
			let mut out = std::io::stderr().lock();
			for line in &capture {
				let _ = writeln!(out, "{tag} | {line}");
			}
		}
		let mut msg = err;
		if let Some(hint) = cgroup_hint(&self.client).await {
			// buildah's `error` field already terminates with a newline;
			// pushing another one on top would leave a blank line before
			// the hint. Trim trailing whitespace first so the separator
			// newline is the only one between the error and the hint.
			msg.truncate(msg.trim_end().len());
			msg.push('\n');
			msg.push_str(&hint);
		}
		ComposeError::Build(msg)
	}
}

/// The slice of `GET /libpod/info` the build-failure hint reads.
///
/// Only `host.cgroupManager` is consulted, but the wrapper struct is named
/// for the endpoint so adding fields (e.g. `host.cgroupVersion`,
/// `host.ociRuntime.name` if a future hint needs them) stays a one-line
/// change. Unknown fields are tolerated by `serde`, so a newer libpod
/// response shape does not break the parse.
#[derive(Deserialize, Default)]
struct LibpodInfo {
	#[serde(default)]
	host: HostInfo,
}

#[derive(Deserialize, Default)]
struct HostInfo {
	#[serde(rename = "cgroupManager", default)]
	cgroup_manager: String,
}

/// On a build failure, ask the libpod API which cgroup manager the daemon
/// is configured to use. Return a one-line hint only when the daemon is set
/// to `systemd` on a host that may not have a user systemd session to talk
/// to it. Any failure of the call itself (connect, transport, parse, the
/// daemon returning a non-2xx, the field missing) is treated as "no hint":
/// the build error reaches the caller unchanged.
async fn cgroup_hint(client: &crate::libpod::Client) -> Option<String> {
	let path = format!("{API_PREFIX}/info");
	let info: LibpodInfo = client.get_json(&path).await.ok()?;
	if info.host.cgroup_manager == "systemd" {
		Some(WSL_CGROUP_HINT.to_string())
	} else {
		None
	}
}

/// The exact hint line appended to a build error when the libpod daemon
/// reports `cgroup_manager = "systemd"`. Phrased as a hint with the
/// condition named, since the same value is correct on an ordinary Linux
/// host with a running user systemd session (#1778).
const WSL_CGROUP_HINT: &str = "hint: if the Podman daemon runs on a host without a user systemd \
	session (for example inside the `podman-machine` WSL distro), setting \
	`cgroup_manager = \"cgroupfs\"` in `~/.config/containers/containers.conf` \
	on that host may unblock the build.";

/// Whether a buildah stream line is the one that carries the new image id.
///
/// Buildah closes a successful build with the full image id on a line of its
/// own: 64 hex digits, nothing else. It is the second-to-last line of the
/// stream, between `Successfully tagged <tag>` and `Successfully built
/// <short-id>` (measured on Podman 5.7, 2026-09-04). The `--> <short-id>`
/// layer markers and `--> Using cache <digest>` carry a prefix and are not
/// matched. A script reading `podup build` from a pipe wants exactly this
/// value, and a terminal does not (#1681).
pub(super) fn parse_image_id_line(line: &str) -> Option<String> {
	let line = line.trim();
	if line.len() != 64 || !line.bytes().all(|b| b.is_ascii_hexdigit()) {
		return None;
	}
	Some(line.to_string())
}

/// Parse one buildah stream line into the row verb it implies, if any.
///
/// On the libpod build stream a `STEP n/m: <instruction>` line arrives once
/// per Dockerfile instruction: the row's verb should reflect where in the
/// build we are. Every other line (`--> 3f3c...`, `COMMIT <tag>`,
/// `Successfully tagged <tag>`) carries no transition of its own; those are
/// routed through `progress::note_for` as tail lines.
#[derive(Default)]
pub(super) struct BuildStreamProgress {
	last_step: Option<(usize, usize)>,
}

impl BuildStreamProgress {
	pub(super) fn new() -> Self {
		Self::default()
	}

	pub(super) fn observe(&mut self, line: &str) -> Option<String> {
		const STEP_PREFIX: &str = "STEP ";
		let line = line.trim_end();
		let rest = line.strip_prefix(STEP_PREFIX)?;
		// `STEP n/m: <instruction>`. The colon separates the counters from
		// the instruction text. Both sides must parse.
		let (counters, _) = rest.split_once(':')?;
		let (cur, total) = counters.split_once('/')?;
		let cur: usize = cur.parse().ok()?;
		let total: usize = total.parse().ok()?;
		self.last_step = Some((cur, total));
		Some(self.format_verb())
	}

	fn format_verb(&self) -> String {
		match self.last_step {
			Some((cur, total)) => format!("Building {cur}/{total}"),
			None => "Building".to_string(),
		}
	}
}
