//! Shared parsing of the label files named by a service.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use crate::compose::types::Service;

use super::{sanitize_kv_pair, MAX_LABEL_FILE_ENTRIES};

/// A problem found while reading a service's label files.
#[derive(Debug)]
pub(crate) enum LabelFileIssue {
	/// The file could not be read.
	Unreadable {
		/// Resolved path of the unreadable file.
		path: PathBuf,
		/// Reason the filesystem refused the read.
		error: io::Error,
	},
	/// One label failed validation and was left out.
	Rejected {
		/// Resolved path of the file holding the rejected label.
		path: PathBuf,
		/// One-based line number of the rejected label.
		line: usize,
		/// Validation rule that rejected the label.
		reason: String,
	},
}

/// Labels read from a service's label files, plus every non-fatal problem
/// found while reading them.
#[derive(Debug, Default)]
pub(crate) struct LabelFileLabels {
	/// Labels that passed validation.
	pub(crate) labels: HashMap<String, String>,
	/// Files that could not be read and labels that failed validation.
	pub(crate) issues: Vec<LabelFileIssue>,
}

/// Read and validate every label file declared by a service.
///
/// Relative paths are resolved against the compose project directory, as the
/// live engine does. A rejected label is left out while the remaining labels
/// stay available: the live path turns the same rejection into an error, but
/// unit generation has no error channel and must still produce a usable unit.
pub(crate) fn read_label_file_labels(service: &Service, base_dir: &Path) -> LabelFileLabels {
	read_label_file_labels_limited(service, base_dir, MAX_LABEL_FILE_ENTRIES)
}

/// Read label files while retaining at most `max_rejections` rejected labels.
///
/// The live path asks for one rejection so it can fail at the first invalid
/// entry, as it did before the reader became shared. Unit generation keeps a
/// bounded number of diagnostics so a hostile file cannot turn every invalid
/// line into an unbounded warning list.
pub(super) fn read_label_file_labels_limited(
	service: &Service,
	base_dir: &Path,
	max_rejections: usize,
) -> LabelFileLabels {
	let mut labels = HashMap::new();
	let mut issues = Vec::new();
	let mut rejections = 0usize;

	'files: for path in service.label_file.to_list() {
		let full = if Path::new(&path).is_absolute() {
			PathBuf::from(&path)
		} else {
			base_dir.join(&path)
		};
		let content = match crate::filesystem::read_to_string_capped(&full) {
			Ok(content) => content,
			Err(error) => {
				issues.push(LabelFileIssue::Unreadable { path: full, error });
				continue;
			}
		};
		for (line_idx, line) in content.lines().enumerate() {
			let trimmed = line.trim();
			if trimmed.is_empty() || trimmed.starts_with('#') {
				continue;
			}
			let mut parts = trimmed.splitn(2, '=');
			let key = parts.next().unwrap_or_default().trim();
			let value = parts.next().unwrap_or_default();
			if key.is_empty() {
				continue;
			}
			if let Err(reason) = sanitize_kv_pair(&mut labels, key, value) {
				issues.push(LabelFileIssue::Rejected {
					path: full.clone(),
					line: line_idx + 1,
					reason: format!("{reason:?}"),
				});
				rejections += 1;
				if rejections >= max_rejections {
					break 'files;
				}
			}
		}
	}

	LabelFileLabels { labels, issues }
}
