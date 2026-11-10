//! Booleans and numbers that a compose file writes as quoted text.
//!
//! `read_only: "true"`, `pids_limit: "100"` and `external: "yes"` are accepted
//! by the reference implementation: a typed field takes the string spelling of
//! its value. serde refuses a string where the field is a `bool` or an
//! integer, so the whole file failed to parse without naming the field. This
//! turns the quoted scalar into the value it spells before the document is
//! typed, in one pass over a table of the typed fields, instead of one custom
//! deserializer per field.
//!
//! Only the exact paths in [`SERVICE_FIELDS`] and [`TOP_LEVEL_FIELDS`] are
//! touched. A key with the same name somewhere that holds free text (a label,
//! an environment value) is left alone, and text that does not spell a value of
//! the field's type is left as it is, so the type error still reports it.

use serde_yaml::{Number, Value};

/// The type a quoted scalar is read as.
#[derive(Clone, Copy)]
enum Kind {
	Bool,
	Int,
	Float,
}

use Kind::{Bool, Float, Int};

/// Typed fields of a service or job, as a path from the entry. `*` stands for
/// every value of a mapping or every item of a list.
const SERVICE_FIELDS: &[(&[&str], Kind)] = &[
	(&["read_only"], Bool),
	(&["privileged"], Bool),
	(&["init"], Bool),
	(&["tty"], Bool),
	(&["stdin_open"], Bool),
	(&["oom_kill_disable"], Bool),
	(&["attach"], Bool),
	(&["use_api_socket"], Bool),
	(&["healthcheck", "disable"], Bool),
	(&["healthcheck", "retries"], Int),
	(&["depends_on", "*", "restart"], Bool),
	(&["depends_on", "*", "required"], Bool),
	(&["env_file", "*", "required"], Bool),
	(&["build", "no_cache"], Bool),
	(&["build", "pull"], Bool),
	(&["build", "privileged"], Bool),
	(&["build", "sbom"], Bool),
	(&["build", "ulimits", "*"], Int),
	(&["build", "ulimits", "*", "soft"], Int),
	(&["build", "ulimits", "*", "hard"], Int),
	(&["develop", "watch", "*", "initial_sync"], Bool),
	(&["post_start", "*", "privileged"], Bool),
	(&["pre_stop", "*", "privileged"], Bool),
	(&["volumes", "*", "read_only"], Bool),
	(&["volumes", "*", "bind", "create_host_path"], Bool),
	(&["volumes", "*", "volume", "nocopy"], Bool),
	(&["volumes", "*", "volume", "noexec"], Bool),
	(&["volumes", "*", "volume", "nosuid"], Bool),
	(&["volumes", "*", "volume", "nodev"], Bool),
	(&["volumes", "*", "tmpfs", "size"], Int),
	(&["ports", "*", "target"], Int),
	(&["networks", "*", "priority"], Int),
	(&["networks", "*", "gw_priority"], Int),
	(&["scale"], Int),
	(&["cpu_shares"], Int),
	(&["cpu_quota"], Int),
	(&["cpu_period"], Int),
	(&["cpu_count"], Int),
	(&["cpu_percent"], Int),
	(&["cpu_rt_runtime"], Int),
	(&["cpu_rt_period"], Int),
	(&["mem_swappiness"], Int),
	(&["pids_limit"], Int),
	(&["oom_score_adj"], Int),
	(&["ulimits", "*"], Int),
	(&["ulimits", "*", "soft"], Int),
	(&["ulimits", "*", "hard"], Int),
	(&["blkio_config", "weight"], Int),
	(&["blkio_config", "weight_device", "*", "weight"], Int),
	(&["deploy", "replicas"], Int),
	(&["deploy", "max_replicas_per_node"], Int),
	(&["deploy", "resources", "limits", "pids"], Int),
	(&["deploy", "resources", "reservations", "pids"], Int),
	(
		&[
			"deploy",
			"resources",
			"reservations",
			"devices",
			"*",
			"count",
		],
		Int,
	),
	(&["deploy", "restart_policy", "max_attempts"], Int),
	(&["deploy", "update_config", "parallelism"], Int),
	(&["deploy", "update_config", "max_failure_ratio"], Float),
	(&["deploy", "rollback_config", "parallelism"], Int),
	(&["deploy", "rollback_config", "max_failure_ratio"], Float),
];

/// Typed fields of an entry of a top-level section.
const TOP_LEVEL_FIELDS: &[(&str, &[(&str, Kind)])] = &[
	(
		"networks",
		&[
			("external", Bool),
			("internal", Bool),
			("attachable", Bool),
			("enable_ipv6", Bool),
			("enable_ipv4", Bool),
		],
	),
	("volumes", &[("external", Bool)]),
	("secrets", &[("external", Bool)]),
	("configs", &[("external", Bool)]),
	("models", &[("context_size", Int)]),
];

/// Read the quoted booleans and numbers of the typed fields of `document` as
/// the values they spell.
pub(in super::super) fn coerce_quoted_scalars(document: &mut Value) {
	for section in ["services", "jobs"] {
		let Some(Value::Mapping(entries)) = document.get_mut(section) else {
			continue;
		};
		for (_, entry) in entries.iter_mut() {
			for (path, kind) in SERVICE_FIELDS {
				coerce_path(entry, path, *kind);
			}
		}
	}
	for (section, fields) in TOP_LEVEL_FIELDS {
		let Some(Value::Mapping(entries)) = document.get_mut(*section) else {
			continue;
		};
		for (_, entry) in entries.iter_mut() {
			for (field, kind) in *fields {
				coerce_path(entry, &[field], *kind);
			}
		}
	}
}

/// Follow `path` from `value` and coerce what it reaches.
fn coerce_path(value: &mut Value, path: &[&str], kind: Kind) {
	let Some((head, rest)) = path.split_first() else {
		coerce_scalar(value, kind);
		return;
	};
	match (value, *head) {
		(Value::Tagged(tagged), _) => coerce_path(&mut tagged.value, path, kind),
		(Value::Mapping(map), "*") => map.iter_mut().for_each(|(_, v)| coerce_path(v, rest, kind)),
		(Value::Sequence(items), "*") => items.iter_mut().for_each(|v| coerce_path(v, rest, kind)),
		(Value::Mapping(map), key) => {
			if let Some(inner) = map.get_mut(key) {
				coerce_path(inner, rest, kind);
			}
		}
		_ => {}
	}
}

fn coerce_scalar(value: &mut Value, kind: Kind) {
	let Value::String(text) = value else { return };
	let parsed = match kind {
		Bool => parse_bool(text).map(Value::Bool),
		Int => parse_int(text).map(Value::Number),
		Float => parse_float(text).map(Value::Number),
	};
	if let Some(parsed) = parsed {
		*value = parsed;
	}
}

/// The spellings of a boolean: `true`, `false`, `yes`, `no`, `y`, `n`, `on` and
/// `off`, in any case. `1` and `t` are not booleans.
fn parse_bool(text: &str) -> Option<bool> {
	match text.to_ascii_lowercase().as_str() {
		"true" | "yes" | "y" | "on" => Some(true),
		"false" | "no" | "n" | "off" => Some(false),
		_ => None,
	}
}

/// A base-10 integer with an optional sign and no surrounding space.
fn parse_int(text: &str) -> Option<Number> {
	text.parse::<i64>()
		.map(Number::from)
		.or_else(|_| text.parse::<u64>().map(Number::from))
		.ok()
}

fn parse_float(text: &str) -> Option<Number> {
	let number = text.parse::<f64>().ok().filter(|n| n.is_finite())?;
	Some(Number::from(number))
}

#[cfg(test)]
#[path = "scalars_tests.rs"]
mod tests;
