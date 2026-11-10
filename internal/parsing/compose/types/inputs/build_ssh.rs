//! `build.ssh`: the list form (`[default, key=/path]`) and the mapping form
//! (`{default: , key: /path}`), both read as the `id[=path]` entries that
//! `podman build --ssh` takes.

use indexmap::IndexMap;
use serde::Deserialize;

pub(super) fn deserialize_ssh<'de, D>(de: D) -> Result<Vec<String>, D::Error>
where
	D: serde::Deserializer<'de>,
{
	#[derive(Deserialize)]
	#[serde(untagged)]
	enum ListOrMap {
		List(Vec<String>),
		Map(IndexMap<String, Option<String>>),
	}
	Ok(match ListOrMap::deserialize(de)? {
		ListOrMap::List(v) => v,
		ListOrMap::Map(m) => m
			.into_iter()
			.map(|(id, path)| match path.filter(|p| !p.is_empty()) {
				Some(path) => format!("{id}={path}"),
				None => id,
			})
			.collect(),
	})
}

#[cfg(test)]
mod tests {
	use super::super::build::BuildConfig;

	fn ssh_of(yaml: &str) -> Vec<String> {
		let b: BuildConfig = serde_yaml::from_str(yaml).unwrap();
		b.ssh().to_vec()
	}

	#[test]
	fn list_form_keeps_ids_and_key_paths() {
		assert_eq!(
			ssh_of("context: .\nssh:\n  - default\n  - mykey=/path/key\n"),
			["default", "mykey=/path/key"]
		);
	}

	#[test]
	fn mapping_form_becomes_id_and_id_equals_path() {
		assert_eq!(
			ssh_of("context: .\nssh:\n  default:\n  mykey: /path/key\n"),
			["default", "mykey=/path/key"]
		);
	}
}
