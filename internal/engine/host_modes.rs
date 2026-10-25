//! Host-binding and privilege-escalation mode warnings.

/// Emit one `tracing::warn!` per active host-binding / privilege-escalation mode
/// across every service in `file`.
///
/// The `config` command uses this to surface the active modes at the default
/// log level (CI logs see them even when the operator never runs `up`). It is
/// deliberately not gated on `--no-warn`; `config` is the "show me what will
/// happen" command, where the warning is the whole point. The live
/// `up`/`create`/`run`/`exec` paths emit the same warnings per-call but honour
/// `--no-warn`.
pub fn surface_host_modes(file: &crate::compose::types::ComposeFile) {
	let jobs = file.jobs.iter().map(|(name, job)| (name, &job.service));
	for (name, service) in file.services.iter().chain(jobs) {
		for w in super::container::check_host_mode(name, service) {
			tracing::warn!("{}", w.message);
		}
	}
}
