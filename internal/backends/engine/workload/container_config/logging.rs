//! The `logging:` block of a service, translated into libpod's `LogConfig`.

use std::collections::HashMap;

use crate::compose::types::LoggingConfig;
use crate::error::ComposeError;
use crate::libpod::types::container::LogConfig;
use crate::size;

/// Default log rotation applied when a compose file does not carry a
/// `logging:` block. The libpod default depends on the host (`journald` on the
/// Podman 6.2.0 host this was measured on, 2026-11-22); pinning `k8s-file`
/// explicitly here stops the answer from drifting between versions and distros. The 10 MB cap is enough for a week of typical service output
/// and small enough that a runaway loop will not exhaust the host. libpod
/// does not honour `max-file` on any path (see `man podman-run`), so it is
/// not part of the default; the user can override by writing their own
/// `logging:` in compose (#1417).
fn default_log_config() -> LogConfig {
	LogConfig {
		driver: Some("k8s-file".into()),
		path: None,
		size: Some(10 * 1024 * 1024),
		options: HashMap::new(),
	}
}

/// Resolve the `logging:` block into libpod's `LogConfig`. An absent compose
/// `logging:` maps to [`default_log_config`] so every container podup creates
/// has a rotation policy without the user having to set one. When the user
/// supplies a block, `max-size` is parsed into the typed [`LogConfig::size`]
/// field libpod actually reads rotation from (passing it inside `options` is
/// silently ignored, #1417); `max-file` is dropped with a warning because
/// libpod does not implement it. A malformed `max-size` is rejected with a
/// `PodmanError::Field` so the user gets a compose-flavoured error instead of
/// a 500 from libpod's JSON unmarshal.
pub(crate) fn build_log_config(
	service_name: &str,
	logging: Option<&LoggingConfig>,
) -> Result<Option<LogConfig>, ComposeError> {
	match logging {
		Some(l) => Ok(Some(translate_user_logging(service_name, l)?)),
		None => Ok(Some(default_log_config())),
	}
}

/// Translate a user-supplied `logging:` block into libpod's [`LogConfig`].
///
/// `max-size` is moved into the typed `size` field and `path` into the typed
/// `path` one: libpod's generator reads both from those fields and ignores them
/// inside `options` (measured on Podman 6.2.0, 2026-11-22: `options.path` left
/// the log at the default place, the typed field wrote it where asked). `path`
/// is read only by the two file drivers, so it also selects the default file
/// driver when the block names none, as a positive `max-size` does, and is
/// dropped with a warning under any other driver. `tag` is dropped with a
/// warning for the `json-file`/`k8s-file` drivers, which refuse it. `max-file`
/// is dropped with a warning: libpod does not implement it and would silently
/// ignore it if forwarded. Any other key is forwarded and reported, since the
/// generator reads nothing else from `options`. The user-supplied value of `max-size` is parsed with the
/// same memory parser used elsewhere in the engine (`10m`, `1g`, plain
/// bytes); a malformed value is rejected with the service field name so the
/// error points at the compose key the user wrote (#1417).
///
/// A `logging:` block without `driver` falls back to the default driver
/// (`k8s-file`) ONLY when the block also sets a POSITIVE `max-size`: only
/// that driver reads the typed size cap, and that is the case a journald
/// host config used to silently override (#1895). A non-positive size
/// (`0`, `-1`) opts out of capping, so pinning `k8s-file` here would
/// silently strip a journald host driver without applying any cap; the
/// field stays None and the host's containers.conf default applies.
/// Without `max-size` and without a named `driver`, both `driver` and
/// `size` are left unset so the host's containers.conf default applies;
/// podup no longer injects a default driver or default size cap. A named
/// driver without `max-size` stays uncapped, just as before.
// Kept out of line: it is cold, and with fat LTO the compiler folded it into the
// container-create path, which grew the release binary by about 12 KB and put it
// over `bench/budgets/binary-budget-mib`.
#[inline(never)]
fn translate_user_logging(
	service_name: &str,
	l: &LoggingConfig,
) -> Result<LogConfig, ComposeError> {
	let mut options = l.options.clone();
	let user_max_size = options.remove("max-size");
	let user_path = options.remove("path");
	let default = default_log_config();
	let size = match user_max_size {
		Some(v) => match size::parse_memory(&v) {
			Some(bytes) => Some(bytes),
			None => {
				return Err(ComposeError::Podman(
					crate::libpod::validate::spec_field_error(
						service_name,
						"logging.options.max-size",
						&v,
						"must be a byte count (e.g. '10m', '1024', '1g'); \
						 libpod rejects an invalid `size` outright",
					),
				));
			}
		},
		None => None,
	};
	// The driver fallback only fires when `max-size` actually constrains
	// rotation. A non-positive size (`0` or `-1`) opts out of capping, so
	// pinning `k8s-file` here would silently strip a journald host driver
	// (#1895 follow-up). An absent `max-size` keeps the host default too.
	let driver = l.driver.clone().or_else(|| {
		if size.is_some_and(|b| b > 0) || user_path.is_some() {
			default.driver
		} else {
			None
		}
	});
	// The generator hands the path to the logger only for the file drivers; for
	// any other it is stored and unused, so the file the user asked for would
	// never appear. A host default that decides the driver is not known here
	// and keeps the path, as it does the tag.
	let path = match (user_path, driver.as_deref()) {
		(Some(_), Some(d)) if !matches!(d, "json-file" | "k8s-file") => {
			tracing::warn!(
				"{service_name}: logging.options.path is ignored; \
				 only the json-file and k8s-file log drivers write a file, not '{d}'"
			);
			None
		}
		(path, _) => path,
	};
	if options.remove("max-file").is_some() {
		tracing::warn!(
			"{service_name}: logging.options.max-file is ignored; \
			 Podman keeps a single log file (truncated at max-size when one is set) \
			 with no rotated history"
		);
	}
	// `json-file` is the name Docker gives the driver, and Podman maps it to its
	// `k8s-file` logger, which refuses a `tag` (Podman 6.2.0 CLI, 2026-11-22: `log tags
	// can only be used with the journald log driver`, for both json-file and
	// k8s-file; the API path at start was last measured on 5.7.0 as
	// `k8s-file doesn't support --log-tag`). Docker accepts and applies the tag.
	// Forwarding it would keep a working compose file from starting, so, like
	// `max-file`, it is dropped with a warning. Other drivers keep theirs.
	if matches!(driver.as_deref(), Some("json-file" | "k8s-file"))
		&& options.remove("tag").is_some()
	{
		tracing::warn!(
			"{service_name}: logging.options.tag is ignored; \
			 Podman's json-file (k8s-file) log driver does not support a tag"
		);
	}
	// libpod's generator reads `tag` from `options` and nothing else; it keeps
	// the rest and never uses it.
	for key in options.keys().filter(|k| k.as_str() != "tag") {
		tracing::warn!(
			"{service_name}: logging.options.{key} is ignored; \
			 Podman's log options are path, max-size and tag, and nothing else is read"
		);
	}
	Ok(LogConfig {
		driver,
		path,
		size,
		options,
	})
}
