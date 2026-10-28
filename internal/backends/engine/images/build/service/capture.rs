//! Bounded tail replay for a failed build on a terminal.

use std::collections::VecDeque;

pub(super) const MAX_CAPTURE_BYTES: usize = 256 * 1024;
pub(super) const MAX_CAPTURE_LINES: usize = 256;

pub(super) struct BuildCapture {
	enabled: bool,
	lines: VecDeque<String>,
	bytes: usize,
}

impl BuildCapture {
	pub(super) fn new(enabled: bool) -> Self {
		Self {
			enabled,
			lines: VecDeque::new(),
			bytes: 0,
		}
	}

	pub(super) fn push(&mut self, line: String) {
		if !self.enabled {
			return;
		}
		let mut start = line.len().saturating_sub(MAX_CAPTURE_BYTES);
		while !line.is_char_boundary(start) {
			start += 1;
		}
		// Copy the retained slice so an oversized record's allocation is freed.
		let line = line[start..].to_string();
		while self.lines.len() >= MAX_CAPTURE_LINES || self.bytes + line.len() > MAX_CAPTURE_BYTES {
			if let Some(old) = self.lines.pop_front() {
				self.bytes -= old.len();
			}
		}
		self.bytes += line.len();
		self.lines.push_back(line);
	}

	pub(super) fn into_lines(self) -> Vec<String> {
		self.lines.into_iter().collect()
	}
}
