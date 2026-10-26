//! The logging configuration of a container: its driver, its rotation cap and
//! its driver options.

use crate::compose::types::Service;
use crate::engine::build_log_config;

use super::super::{quote_podman_arg_value, sorted_label_pairs, Section};

/// Emit the `LogDriver=` line and the rotation/driver options for the service.
pub(in super::super) fn emit_logging(
	name: &str,
	service: &Service,
	container: &mut Section,
	warnings: &mut Vec<String>,
) {
	// `build_log_config` substitutes the rotation default when `service.logging`
	// is None, so an absent `logging:` block in compose still produces a
	// `LogDriver=` set and `--log-opt` rotation policy on the generated
	// unit (#1354). The render path is the same as the live engine's, so
	// `up` and `generate quadlet` produce equivalent rotation. The
	// options are emitted through `PodmanArgs=` because `LogOpt=` is a
	// Podman 5.2.0 key and the floor is 5.0; see `emit_log_config`.
	match build_log_config(name, service.logging.as_ref()) {
		Ok(Some(logging)) => emit_log_config(container, logging),
		Ok(None) => {}
		Err(e) => {
			// A malformed `max-size` does not abort generation; render the
			// default rotation and surface the parse error as a warning so the
			// user sees it before `up` rejects the same value outright.
			warnings.push(format!("{name}: {e}"));
			let logging = build_log_config(name, None)
				.expect("default log config is infallible")
				.expect("default returns Some");
			emit_log_config(container, logging);
		}
	}
}

/// Render a resolved [`LogConfig`] onto a Quadlet `[Container]` section.
///
/// Quadlet units run through the podman CLI, which reads rotation from
/// `--log-opt max-size=`, not from the typed `size` field the libpod API
/// takes. #1417 moved `max-size` out of `options` into that typed field for
/// the live path, and rendering `options` alone here dropped rotation from
/// every generated unit: measured before this helper, a default project
/// emitted `LogDriver=k8s-file` and no `LogOpt` at all, where it used to
/// carry `LogOpt=max-size=10m`.
///
/// The byte count is emitted verbatim; `podman run --log-opt
/// max-size=10485760` was measured to produce the same `10.49MB` cap as
/// `max-size=10m` on Podman 5.7.0, so no suffix has to be reconstructed.
///
/// Each entry is emitted through `PodmanArgs=` (`--log-opt=`) rather than
/// as the native `LogOpt=` Quadlet key, because `LogOpt=` is a Podman 5.2.0
/// addition; the supported floor is 5.0.
pub(in super::super) fn emit_log_config(
	container: &mut Section,
	logging: crate::libpod::types::container::LogConfig,
) {
	if let Some(driver) = &logging.driver {
		container.add("LogDriver", driver.clone());
	}
	// `LogOpt=` was added to Quadlet in Podman 5.2.0. The supported floor
	// is 5.0, so route the rotation cap and each user option through
	// `PodmanArgs=` as `--log-opt`, like the other escape-hatch flags. Both
	// the key and the value are quoted (#1734): a hostile value carrying
	// whitespace must not become two argv elements, and the key half has
	// the same hole on the other side of the `=`.
	if let Some(size) = logging.size {
		container.add(
			"PodmanArgs",
			format!(
				"--log-opt={}",
				quote_podman_arg_value(&format!("max-size={size}"))
			),
		);
	}
	for (key, val) in sorted_label_pairs(logging.options) {
		// An empty value (`options: { tag: "" }`) would render as
		// `--log-opt=tag=`, which podman rejects. The native `LogOpt=`
		// key omits empty values, so the podman-args route has to skip
		// them too. An empty key would render as `--log-opt==value`,
		// equally malformed; skip it for the same reason.
		if key.is_empty() || val.is_empty() {
			continue;
		}
		let line = format!("{key}={val}");
		container.add(
			"PodmanArgs",
			format!("--log-opt={}", quote_podman_arg_value(&line)),
		);
	}
}
