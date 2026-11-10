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
#[path = "build_ssh_tests.rs"]
mod tests;
