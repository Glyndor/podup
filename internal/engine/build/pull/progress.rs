use std::collections::HashSet;

/// Whether the per-blob `start` calls should fire for this process. Mirrors the
/// gate `internal::ui::progress::live_terminal_colored` uses to decide whether
/// to open a live region: stderr must be a tty, and the colour choice must not
/// be `Never`.
///
/// Centralised here so the production predicate and the gate that protects the
/// plain sink stay in lockstep. A `start` call that goes through when the
/// live region is off would write one line per blob to a CI log, which is
/// exactly what `a_piped_pull_prints_only_pulling_and_pulled` says must not
/// happen.
pub(super) fn stderr_supports_live() -> bool {
	use std::io::IsTerminal;
	std::io::stderr().is_terminal() && crate::ui::stderr_colored()
}

/// Parse one libpod pull stream line into the row verb it implies, if any.
///
/// On Podman 5.7.0 the `/libpod/images/pull` stream carries no byte counts:
/// one `{"stream":"Copying blob sha256:<digest>"}` per layer, plus a
/// `Copying config <digest>` once the manifest has been read. A blob already in
/// storage is skipped by libpod without a `Copying blob` line, and a blob that
/// fails mid-transfer is reported again under the same digest on retry, so the
/// count is of layers that are actually copied.
///
/// Kept as a standalone struct so a unit test can feed it three lines and read
/// the verbs back out, which is the only way to assert what the row actually
/// said without an end-to-end harness.
#[derive(Default)]
pub(super) struct PullStreamProgress {
	/// Distinct blob digests seen on `Copying blob` lines, including retries.
	blobs: HashSet<String>,
	/// Number of `Copying blob` lines observed (raw count, so retries move this
	/// while `blobs.len()` stays put).
	blob_lines: usize,
	/// Whether the `Copying config` line has arrived, switching the verb from
	/// the `Pulling N layers` form to `Pulling done/total`.
	manifest_seen: bool,
}

impl PullStreamProgress {
	pub(super) fn new() -> Self {
		Self::default()
	}

	/// Observe one `stream` line. Returns the verb to render on the row, or
	/// `None` for lines that carry no transition (status text, the
	/// `Copying config` line carries a transition of its own).
	pub(super) fn observe(&mut self, line: &str) -> Option<String> {
		const BLOB_PREFIX: &str = "Copying blob ";
		const CONFIG_PREFIX: &str = "Copying config ";
		let line = line.trim_end();
		if let Some(rest) = line.strip_prefix(BLOB_PREFIX) {
			let digest = rest.split_whitespace().next().unwrap_or("");
			self.blobs.insert(digest.to_string());
			self.blob_lines += 1;
			Some(self.format_verb())
		} else if line.starts_with(CONFIG_PREFIX) {
			self.manifest_seen = true;
			Some(self.format_verb())
		} else {
			None
		}
	}

	fn format_verb(&self) -> String {
		if self.manifest_seen {
			format!("Pulling {}/{}", self.blob_lines, self.blobs.len())
		} else {
			match self.blobs.len() {
				1 => "Pulling 1 layer".to_string(),
				n => format!("Pulling {n} layers"),
			}
		}
	}
}
