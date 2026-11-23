//! A mode written out the way it can be read back.
//!
//! `config` prints the model it resolved, and the model holds a mode as its
//! permission bits, which print as a decimal number: 0o644 is `420`, and read
//! again, `420` is a different mode (see [`super::mode`]). The printed document
//! is meant to be fed back, so the mode of a secret, a config and a tmpfs is
//! printed as a string with its leading zero, as `docker compose config` prints
//! it: `"0644"`. That spelling reads back as the same mode wherever it is read.

use serde_json::Value as Json;
use serde_yaml::Value as Yaml;

/// Whether the keys leading to a node end where a `mode:` of a secret or config
/// reference, or of a tmpfs, stands. `[]` stands for a list item.
fn is_mode_place(path: &[&str]) -> bool {
	let n = path.len();
	if n < 2 || path[n - 1] != "mode" {
		return false;
	}
	path[n - 2] == "tmpfs"
		|| (n >= 3 && path[n - 2] == "[]" && matches!(path[n - 3], "secrets" | "configs"))
}

fn text(bits: u64) -> String {
	format!("0{bits:o}")
}

/// Replace the number at every mode place of a YAML document by its text.
pub(crate) fn modes_as_text_yaml(value: &mut Yaml) {
	fn walk(value: &mut Yaml, path: &mut Vec<String>) {
		match value {
			Yaml::Tagged(tagged) => walk(&mut tagged.value, path),
			Yaml::Mapping(map) => {
				for (key, child) in map.iter_mut() {
					let Some(name) = key.as_str() else {
						continue;
					};
					path.push(name.to_string());
					let keys: Vec<&str> = path.iter().map(String::as_str).collect();
					if is_mode_place(&keys) {
						if let Yaml::Number(n) = child {
							if let Some(bits) = n.as_u64() {
								*child = Yaml::String(text(bits));
							}
						}
					}
					walk(child, path);
					path.pop();
				}
			}
			Yaml::Sequence(seq) => {
				path.push("[]".to_string());
				for child in seq.iter_mut() {
					walk(child, path);
				}
				path.pop();
			}
			_ => {}
		}
	}
	walk(value, &mut Vec::new());
}

/// The same for a JSON document.
pub(crate) fn modes_as_text_json(value: &mut Json) {
	fn walk(value: &mut Json, path: &mut Vec<String>) {
		match value {
			Json::Object(map) => {
				for (key, child) in map.iter_mut() {
					path.push(key.clone());
					let keys: Vec<&str> = path.iter().map(String::as_str).collect();
					if is_mode_place(&keys) {
						if let Some(bits) = child.as_u64() {
							*child = Json::String(text(bits));
						}
					}
					walk(child, path);
					path.pop();
				}
			}
			Json::Array(items) => {
				path.push("[]".to_string());
				for child in items.iter_mut() {
					walk(child, path);
				}
				path.pop();
			}
			_ => {}
		}
	}
	walk(value, &mut Vec::new());
}
