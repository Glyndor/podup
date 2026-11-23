//! The logging configuration of a container: its driver, its rotation cap and
//! its driver options.

use crate::compose::types::Service;
use crate::engine::build_log_config;

use super::super::{sorted_label_pairs, Section};

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
	// options are written as `LogOpt=` lines; see `emit_log_config`.
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
/// The byte count is emitted verbatim: measured on Podman 6.2.0 (2026-11-22),
/// `max-size=10485760` stores the 10 MiB cap (`10.49MB`) that `up` sets, so no
/// suffix has to be reconstructed.
///
/// Each entry is one `LogOpt=` line.
pub(in super::super) fn emit_log_config(
	container: &mut Section,
	logging: crate::libpod::types::container::LogConfig,
) {
	if let Some(driver) = &logging.driver {
		container.add("LogDriver", driver.clone());
	}
	// `Section::add` quotes a `LogOpt=` value that carries whitespace or a
	// quote, so a hostile value cannot become two `--log-opt` arguments, and the
	// key half has the same protection on the other side of the `=`.
	if let Some(path) = &logging.path {
		container.add_native("LogOpt", "--log-opt=", &format!("path={path}"));
	}
	if let Some(size) = logging.size {
		container.add_native("LogOpt", "--log-opt=", &format!("max-size={size}"));
	}
	for (key, val) in sorted_label_pairs(logging.options) {
		// An empty value (`options: { tag: "" }`) would render as `tag=`, which
		// podman rejects. An empty key would render as `=value`, equally
		// malformed; skip both.
		if key.is_empty() || val.is_empty() {
			continue;
		}
		container.add_native("LogOpt", "--log-opt=", &format!("{key}={val}"));
	}
}
