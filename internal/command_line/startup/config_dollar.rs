//! `$` in the output of `config`.
//!
//! By the time `config` prints, the model holds interpolated values, so a `$`
//! in one is a literal dollar sign. In a compose file a literal dollar sign is
//! written `$$`, which is what Docker Compose prints (a value `$HOME` as
//! `$$HOME`), and what makes the output read back as the same model. Printing a
//! bare `$` made `config | podup -f - config` interpolate it a second time.
//! `--no-interpolate` leaves the text as it was written and is not escaped.

/// Double every `$` in each string value of `v`. Keys are not touched.
pub(crate) fn escape_yaml(v: &mut serde_yaml::Value) {
	match v {
		serde_yaml::Value::String(s) => escape(s),
		serde_yaml::Value::Sequence(items) => items.iter_mut().for_each(escape_yaml),
		serde_yaml::Value::Mapping(map) => map.iter_mut().for_each(|(_, value)| escape_yaml(value)),
		serde_yaml::Value::Tagged(tagged) => escape_yaml(&mut tagged.value),
		_ => {}
	}
}

/// [`escape_yaml`] for the JSON rendering.
pub(crate) fn escape_json(v: &mut serde_json::Value) {
	match v {
		serde_json::Value::String(s) => escape(s),
		serde_json::Value::Array(items) => items.iter_mut().for_each(escape_json),
		serde_json::Value::Object(map) => map.values_mut().for_each(escape_json),
		_ => {}
	}
}

fn escape(s: &mut String) {
	if s.contains('$') {
		*s = s.replace('$', "$$");
	}
}

#[cfg(test)]
#[path = "config_dollar_tests.rs"]
mod tests;
