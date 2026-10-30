/// Recursively resolve YAML merge keys (`<<: *anchor`) in a `Value` tree.
///
/// This resolves them itself rather than through the parser's `apply_merge()`.
/// Merge semantics: keys from the anchor fill in only where the child has no value.
pub(super) fn apply_merge_keys(value: &mut serde_yaml::Value) {
	match value {
		serde_yaml::Value::Mapping(mapping) => {
			for v in mapping.values_mut() {
				apply_merge_keys(v);
			}
			let merge_key = serde_yaml::Value::String("<<".to_string());
			if let Some(merge_val) = mapping.remove(&merge_key) {
				let bases: Vec<serde_yaml::Mapping> = match merge_val {
					serde_yaml::Value::Mapping(m) => vec![m],
					serde_yaml::Value::Sequence(seq) => seq
						.into_iter()
						.filter_map(|v| match v {
							serde_yaml::Value::Mapping(m) => Some(m),
							_ => None,
						})
						.collect(),
					_ => vec![],
				};
				for base in bases {
					for (k, v) in base {
						if !mapping.contains_key(&k) {
							mapping.insert(k, v);
						}
					}
				}
			}
		}
		serde_yaml::Value::Sequence(seq) => {
			for v in seq.iter_mut() {
				apply_merge_keys(v);
			}
		}
		// A tagged section or service (`services: !override {...}`) holds its
		// mapping behind the tag; the tag stays for the override semantics.
		serde_yaml::Value::Tagged(tagged) => apply_merge_keys(&mut tagged.value),
		_ => {}
	}
}
