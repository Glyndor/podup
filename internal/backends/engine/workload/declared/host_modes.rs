//! Host-binding and privilege-escalation mode warnings.

/// Emit one `tracing::warn!` per active host-binding / privilege-escalation mode
/// across every service in `file`.
///
/// The `config` command uses this to surface the active modes at the default
/// log level (CI logs see them even when the operator never runs `up`). They
/// are security warnings, so `--no-warn` does not silence them here or on the
/// live `up`/`create`/`run`/`exec` paths, which emit the same warnings per call.
pub fn surface_host_modes(file: &crate::compose::types::ComposeFile) {
	let jobs = file.jobs.iter().map(|(name, job)| (name, &job.service));
	for (name, service) in file.services.iter().chain(jobs) {
		for w in crate::backends::engine::workload::container::check_host_mode(name, service) {
			tracing::warn!("{}", crate::compose::jobs::relabel(file, w.message));
		}
	}
}
