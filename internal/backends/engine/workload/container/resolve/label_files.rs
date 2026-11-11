use std::path::Path;

use crate::compose::types::Service;

/// Fold the labels a service's `label_file:` entries produce into the config
/// hash.
///
/// The service definition carries only the file names, so without this an
/// edit to a `label_file:` left the hash equal and `up` kept the container
/// with the labels it had. The labels are folded the way the create path
/// reads them (same parser, so comments and blank lines do not count), in key
/// order so the fold is stable. A file the create path cannot read or parse
/// contributes nothing: that path reports it, and hashing must not turn it
/// into a second error. A service without `label_file:` folds nothing, so the
/// hash of every existing container is unchanged by this.
pub(super) fn fold_label_files(hasher: &mut sha2::Sha256, service: &Service, base_dir: &Path) {
	use sha2::Digest;
	let Ok(labels) = super::super::fields::build_label_file_labels(service, base_dir) else {
		return;
	};
	let mut sorted: Vec<_> = labels.iter().collect();
	sorted.sort();
	for (key, value) in sorted {
		hasher.update(b"label_file");
		hasher.update((key.len() as u64).to_le_bytes());
		hasher.update(key.as_bytes());
		hasher.update((value.len() as u64).to_le_bytes());
		hasher.update(value.as_bytes());
	}
}
