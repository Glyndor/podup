//! Per-line log prefixing for `docker compose logs`-style multi-service output.

use std::io::Write;

/// Cap on the buffered partial line. A container that emits a very long run
/// with no newline — a `\r`-updated progress bar, binary output, a pathological
/// single line — must not grow `pending` without bound. At this size the
/// partial is flushed as its own prefixed line (as docker does) rather than
/// held in memory forever.
const MAX_PENDING: usize = 64 * 1024;

/// Tags each complete log line with `{label} | `, the way `docker compose logs`
/// labels multi-service output. Bytes arrive as stream frames that may split a
/// line across frames, so a partial line is buffered until its newline arrives
/// (up to [`MAX_PENDING`]).
pub(super) struct LinePrefixer {
	label: String,
	pending: Vec<u8>,
}

impl LinePrefixer {
	/// Build a prefixer for `label`. `prefix` gates whether any `{label} | ` is
	/// emitted at all (`logs --no-log-prefix`); `allow_color` gates the colour of
	/// the prefix (`logs --no-color`), still subject to stdout being a colour sink.
	pub(super) fn new(label: &str, prefix: bool, allow_color: bool) -> Self {
		// `--no-log-prefix`: emit the bare line with no `{label} | ` tag.
		if !prefix {
			return Self {
				label: String::new(),
				pending: Vec::new(),
			};
		}
		// Colour the whole prefix with the service's stable colour so aggregated
		// multi-service output is easy to scan. Gated on stdout being a colour sink
		// (a raw write anstream does not strip for us) and on `--no-color`.
		let plain = format!("{label}  | ");
		let label = crate::ui::paint(
			crate::ui::service_style(label),
			&plain,
			allow_color && crate::ui::stdout_colored(),
		);
		Self {
			label,
			pending: Vec::new(),
		}
	}

	/// Buffer `chunk` and write every complete line it now completes.
	pub(super) fn write(&mut self, out: &mut impl Write, chunk: &[u8]) {
		self.pending.extend_from_slice(chunk);
		while let Some(nl) = self.pending.iter().position(|&b| b == b'\n') {
			let _ = out.write_all(self.label.as_bytes());
			let _ = out.write_all(&self.pending[..=nl]);
			self.pending.drain(..=nl);
		}
		// The remaining bytes are a partial line with no newline yet. Bound it:
		// a container spewing without a newline (a `\r` progress bar, binary
		// data) would otherwise grow `pending` without limit. Break the
		// over-long partial into its own prefixed line and start fresh.
		if self.pending.len() >= MAX_PENDING {
			let _ = out.write_all(self.label.as_bytes());
			let _ = out.write_all(&self.pending);
			let _ = out.write_all(b"\n");
			self.pending.clear();
		}
		let _ = out.flush();
	}

	/// Flush a trailing line that never received a newline (e.g. at stream end).
	pub(super) fn flush_tail(&mut self, out: &mut impl Write) {
		if !self.pending.is_empty() {
			let _ = out.write_all(self.label.as_bytes());
			let _ = out.write_all(&self.pending);
			let _ = out.write_all(b"\n");
			let _ = out.flush();
			self.pending.clear();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::LinePrefixer;

	#[test]
	fn line_prefixer_tags_lines_and_buffers_partials() {
		let mut p = LinePrefixer::new("web", true, false);
		let mut out: Vec<u8> = Vec::new();
		p.write(&mut out, b"hello\nwor");
		// The complete line is tagged; the partial "wor" waits for its newline.
		assert_eq!(out, b"web  | hello\n");
		p.write(&mut out, b"ld\n");
		assert_eq!(out, b"web  | hello\nweb  | world\n");
	}

	#[test]
	fn line_prefixer_flush_tail_emits_unterminated_line() {
		let mut p = LinePrefixer::new("db", true, false);
		let mut out: Vec<u8> = Vec::new();
		p.write(&mut out, b"partial");
		assert!(out.is_empty(), "a line with no newline is held back");
		p.flush_tail(&mut out);
		assert_eq!(out, b"db  | partial\n");
	}

	#[test]
	fn line_prefixer_bounds_a_newlineless_flood() {
		use super::MAX_PENDING;
		let mut p = LinePrefixer::new("web", true, false);
		let mut out: Vec<u8> = Vec::new();
		// Feed more than the cap with no newline in sight, in small chunks.
		let chunk = vec![b'x'; 4096];
		for _ in 0..((MAX_PENDING / chunk.len()) + 2) {
			p.write(&mut out, &chunk);
		}
		// The partial was flushed as a prefixed line instead of being buffered
		// unbounded, and nothing is left pending beyond the last sub-cap chunk.
		assert!(
			!out.is_empty(),
			"the over-long partial was emitted, not held"
		);
		assert!(
			p.pending.len() < MAX_PENDING,
			"pending stays bounded under the cap, was {}",
			p.pending.len()
		);
		assert!(
			out.starts_with(b"web  | "),
			"the flushed partial is prefixed"
		);
	}

	#[test]
	fn line_prefixer_no_prefix_emits_bare_lines() {
		// `--no-log-prefix`: lines pass through with no `{label} | ` tag.
		let mut p = LinePrefixer::new("web", false, false);
		let mut out: Vec<u8> = Vec::new();
		p.write(&mut out, b"hello\n");
		assert_eq!(out, b"hello\n");
		p.write(&mut out, b"tail");
		p.flush_tail(&mut out);
		assert_eq!(out, b"hello\ntail\n");
	}
}
