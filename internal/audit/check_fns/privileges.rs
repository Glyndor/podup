use podup::compose::types::{ComposeFile, Service};

use super::{finding, Finding};

// The audit must read the runtime value a `security_opt` entry resolves to,
// not the raw compose-side text (#1743). `podup::effective_no_new_privileges`
// calls the engine's own `parse_security_opts` so the two cannot drift again.
use podup::effective_no_new_privileges;

/// `privileged: true`, grants extended host privileges that bypass the
/// default capability set. Under rootless Podman the effect is reduced but
/// the flag still means "give me more than the baseline"; it's never
/// incidental.
pub fn check_privileged(name: &str, service: &Service, _file: &ComposeFile) -> Vec<Finding> {
	if service.privileged == Some(true) {
		vec![finding(
			name,
			"privileged",
			"privileged: true grants extended host privileges",
		)]
	} else {
		Vec::new()
	}
}

/// `cap_add: [SYS_ADMIN]`, `cap_add: [SYS_MODULE]`, or any other capability
/// in the curated dangerous list. The runtime honours exactly what was
/// asked for, so reading the resolved value cannot narrow the audit; this
/// check exists because the spec has no opinion about which capabilities
/// are dangerous, and the operator who ships `--strict` in CI needs one
/// (`#1743`).
pub fn check_dangerous_capability(
	name: &str,
	service: &Service,
	_file: &ComposeFile,
) -> Vec<Finding> {
	let mut out = Vec::new();
	for cap in &service.cap_add {
		if let Some(reason) = dangerous_capability_reason(&normalized_capability(cap)) {
			out.push(finding(
				name,
				"dangerous_capability",
				&format!("cap_add: {cap}: {reason}"),
			));
		}
	}
	out
}

/// Returns the reason a normalised capability (`CAP_` stripped,
/// upper-cased) is on the dangerous list, or `None` when it is not. The
/// list is curated against the CIS Docker Benchmark and the Podman
/// hardening notes: each entry either grants a path to host-level
/// compromise (kernel / syscall / audit), breaks the container's file
/// or device isolation, or subverts networking in a way that lets one
/// container attack another. `SYS_ADMIN` and `ALL` are the obvious
/// root-equivalents; the rest are the narrower capabilities an attacker
/// chains into the same outcome.
fn dangerous_capability_reason(cap: &str) -> Option<&'static str> {
	Some(match cap {
		"ALL" => "every capability, equivalent to root",
		"SYS_ADMIN" => "broad kernel administration, effectively root",
		"SYS_MODULE" => "load or unload kernel modules (root-equivalent)",
		"DAC_READ_SEARCH" => "bypass file read and directory search permission checks",
		"SYS_RAWIO" => "raw I/O port access, can crash or compromise the host kernel",
		"SYS_PTRACE" => "ptrace any process, including those in other containers",
		"NET_ADMIN" => "arbitrary network interface and routing changes",
		"SYS_BOOT" => "reboot or halt the host from inside the container",
		"MKNOD" => "create device nodes that mimic host block devices",
		"SYSLOG" => "read the kernel ring buffer (information disclosure)",
		"AUDIT_CONTROL" => "configure the kernel audit subsystem",
		"AUDIT_WRITE" => "tamper with the audit log",
		"SETFCAP" => "set arbitrary file capabilities on host binaries",
		_ => return None,
	})
}

/// `cap_drop` without `ALL`, the service can inherit a broader capability
/// set than it asked to drop. Spec asks for `ALL` (so the service starts
/// from nothing and opts back in via `cap_add:`).
pub fn check_no_cap_drop_all(name: &str, service: &Service, _file: &ComposeFile) -> Vec<Finding> {
	if !service
		.cap_drop
		.iter()
		.any(|c| normalized_capability(c) == "ALL")
	{
		vec![finding(
			name,
			"no_cap_drop_all",
			"cap_drop does not contain ALL: the service keeps the runtime's default capability set",
		)]
	} else {
		Vec::new()
	}
}

/// `security_opt` without `no-new-privileges:true`. Podman spells it
/// `no-new-privileges` (no `:true`); both spellings are accepted. The check
/// asks the engine's own `parse_security_opts` for the resolved value
/// (`#1743`): the engine splits on `:` or `=` and last-wins, so an audit
/// that splits on `:` only disagrees with the runtime on every docker-form
/// `=true` and on contradictory multi-entry lists like
/// `[no-new-privileges:true, no-new-privileges:false]`.
pub fn check_no_new_privileges_off(
	name: &str,
	service: &Service,
	_file: &ComposeFile,
) -> Vec<Finding> {
	if effective_no_new_privileges(service) == Some(true) {
		Vec::new()
	} else {
		vec![finding(
			name,
			"no_new_privileges_off",
			"security_opt is missing no-new-privileges:true: setuid binaries may regain privileges",
		)]
	}
}
/// `CAP_SYS_ADMIN`, `sys_admin` and `SYS_ADMIN` name the same capability;
/// compose files carry all three spellings.
fn normalized_capability(cap: &str) -> String {
	let upper = cap.trim().to_ascii_uppercase();
	upper.strip_prefix("CAP_").unwrap_or(&upper).to_string()
}
