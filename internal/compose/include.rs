//! `include:` directive — merging externally included compose files.
//!
//! Included files are merged into the parent: services, volumes, networks,
//! secrets, and configs from the included file are added only if the key does
//! not already exist in the parent (parent wins on conflict).

use super::types::ComposeFile;

/// Merge `other` into `target`.
///
/// Services / volumes / networks / secrets / configs from `other` are added;
/// existing entries in `target` win on conflict (parent file overrides included content).
pub(super) fn merge_compose_file(target: &mut ComposeFile, other: ComposeFile) {
	for (k, v) in other.services {
		target.services.entry(k).or_insert(v);
	}
	for (k, v) in other.volumes {
		target.volumes.entry(k).or_insert(v);
	}
	for (k, v) in other.networks {
		target.networks.entry(k).or_insert(v);
	}
	for (k, v) in other.secrets {
		target.secrets.entry(k).or_insert(v);
	}
	for (k, v) in other.configs {
		target.configs.entry(k).or_insert(v);
	}
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::super::types::Service;
	use super::*;

	fn svc(image: &str) -> Service {
		Service {
			image: Some(image.to_string()),
			..Default::default()
		}
	}

	#[test]
	fn merge_adds_service_from_other() {
		let mut target = ComposeFile::default();
		let mut other = ComposeFile::default();
		other.services.insert("db".to_string(), svc("postgres:16"));
		merge_compose_file(&mut target, other);
		assert!(target.services.contains_key("db"));
	}

	#[test]
	fn merge_parent_wins_on_service_conflict() {
		let mut target = ComposeFile::default();
		target.services.insert("web".to_string(), svc("nginx:1.25"));
		let mut other = ComposeFile::default();
		other.services.insert("web".to_string(), svc("nginx:1.24"));
		merge_compose_file(&mut target, other);
		assert_eq!(target.services["web"].image.as_deref(), Some("nginx:1.25"));
	}

	#[test]
	fn merge_adds_volume_from_other() {
		let mut target = ComposeFile::default();
		let mut other = ComposeFile::default();
		other.volumes.insert("data".to_string(), None);
		merge_compose_file(&mut target, other);
		assert!(target.volumes.contains_key("data"));
	}

	#[test]
	fn merge_parent_wins_on_volume_conflict() {
		let mut target = ComposeFile::default();
		target.volumes.insert("data".to_string(), None);
		let mut other = ComposeFile::default();
		other.volumes.insert("data".to_string(), None);
		merge_compose_file(&mut target, other);
		assert_eq!(target.volumes.len(), 1);
	}

	#[test]
	fn merge_adds_network_from_other() {
		let mut target = ComposeFile::default();
		let mut other = ComposeFile::default();
		other.networks.insert("backend".to_string(), None);
		merge_compose_file(&mut target, other);
		assert!(target.networks.contains_key("backend"));
	}

	#[test]
	fn merge_empty_other_is_noop() {
		let mut target = ComposeFile::default();
		target.services.insert("web".to_string(), svc("nginx:1.25"));
		let other = ComposeFile::default();
		merge_compose_file(&mut target, other);
		assert_eq!(target.services.len(), 1);
	}
}
