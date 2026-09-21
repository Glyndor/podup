//! Hardening checks: registry of every check the audit carries, plus the
//! [`run_checks`] dispatch that walks it. `CHECK_REGISTRY` is the single source
//! of truth: `audit --list-checks` iterates it to print ids and descriptions,
//! and `audit_file` iterates the same to invoke each `run` function. A check
//! function in `check_fns.rs` constructs its findings with a hand-written
//! `&'static str` for `id`; a unit test in `audit_tests.rs` pins the
//! equivalence, so the listing and the run cannot drift: an id the listing
//! promises is one a finding can emit, and an id a finding emits is one the
//! listing can name.
//!
//! Every check id is a stable snake_case string. Renaming one is a breaking
//! change for consumers; add new ids instead. Adding a check is one entry in
//! the registry plus one function in `check_fns.rs`.

use podup::compose::types::{ComposeFile, Service};

use super::Finding;

#[path = "check_fns.rs"]
mod check_fns;

#[path = "check_sensitive_bind.rs"]
mod check_sensitive_bind;

pub(super) use check_sensitive_bind::check_sensitive_bind_mount;

pub(super) use check_fns::{
	check_dangerous_capability, check_host_namespace, check_no_cap_drop_all, check_no_memory_limit,
	check_no_new_privileges_off, check_no_pids_limit, check_no_userns,
	check_port_published_on_all_interfaces, check_privileged, check_secret_in_environment,
	check_unpinned_image, check_writable_root,
};

// `segments` is a helper only consumed from `check_fns` itself during the
// build and from `checks_secret_env_tests.rs` via `super::segments` under
// `cfg(test)`. Re-export it under the same gate so the non-test build stays
// warning-clean: any drift in the helper still hits the test path that has
// always read it through this module.
#[cfg(test)]
pub(super) use check_fns::segments;

/// Every check has the same shape: take the service name and the parsed
/// service plus the whole file (for cross-service context), return the
/// findings the service raises under this check. Pinned here so the
/// `CHECK_REGISTRY` stays trivial and a new check is one entry plus one
/// function.
type CheckFn = fn(&str, &Service, &ComposeFile) -> Vec<Finding>;

/// One entry in [`CHECK_REGISTRY`]: the stable check id, its one-line
/// description for `audit --list-checks`, and the run function that emits
/// findings carrying that id. Carried together so the listing path and the
/// audit dispatch path cannot drift; if a check function changes its emitted
/// id, a unit test fails before the registry reaches the binary.
pub(super) struct CheckDescriptor {
	/// Stable snake_case id the run function emits and the listing prints.
	pub id: &'static str,
	/// One-line description printed by `audit --list-checks` (table and JSON).
	/// Plain prose, no markup; the field is what shows when an integrator
	/// diffs the listing between releases.
	pub description: &'static str,
	/// The run function called by `run_checks` for one service.
	pub run: CheckFn,
}

/// Apply every registered check to one service, returning the union of all
/// findings. Iterates [`CHECK_REGISTRY`] so the listing path and the audit
/// dispatch path always enumerate the same set of checks: a check listed by
/// `audit --list-checks` is one the audit path will run, and vice versa.
///
/// `service_name` is the compose key; it is folded into each finding so the
/// renderer can group by service.
pub(super) fn run_checks(
	service_name: &str,
	service: &Service,
	file: &ComposeFile,
) -> Vec<Finding> {
	let mut out = Vec::new();
	for check in CHECK_REGISTRY {
		out.extend((check.run)(service_name, service, file));
	}
	out
}

/// Every check the audit carries. Single source of truth: `audit --list-checks`
/// iterates this to print ids+descriptions; `run_checks` iterates the same to
/// invoke each `run` function. Adding a check is one entry plus one function.
///
/// Each `id` here must match the `Finding.check` the corresponding `run`
/// function emits; the drift test in `audit_tests.rs` pins this without
/// coupling to the listing renderer, so a hand-edited id that goes one way
/// and not the other flips the test before the binary carries the lie.
pub(super) const CHECK_REGISTRY: &[CheckDescriptor] = &[
	CheckDescriptor {
		id: "privileged",
		description: "privileged: true grants extended host privileges.",
		run: check_privileged,
	},
	CheckDescriptor {
		id: "host_namespace",
		description: "a host-binding namespace mode shares the host's or another container's namespace.",
		run: check_host_namespace,
	},
	CheckDescriptor {
		id: "dangerous_capability",
		description: "cap_add carries a capability from the dangerous list (kernel admin, audit, networking, device nodes).",
		run: check_dangerous_capability,
	},
	CheckDescriptor {
		id: "writable_root",
		description: "read_only is not true: the container's root filesystem is writable.",
		run: check_writable_root,
	},
	CheckDescriptor {
		id: "no_cap_drop_all",
		description: "cap_drop does not contain ALL: the service keeps the runtime's default capability set.",
		run: check_no_cap_drop_all,
	},
	CheckDescriptor {
		id: "no_new_privileges_off",
		description: "security_opt is missing no-new-privileges:true: setuid binaries may regain privileges.",
		run: check_no_new_privileges_off,
	},
	CheckDescriptor {
		id: "no_pids_limit",
		description: "pids_limit is not set: a fork bomb can exhaust the host's process table.",
		run: check_no_pids_limit,
	},
	CheckDescriptor {
		id: "no_memory_limit",
		description: "neither mem_limit nor deploy.resources.limits.memory is parseable: a leak can OOM the host.",
		run: check_no_memory_limit,
	},
	CheckDescriptor {
		id: "no_userns",
		description: "userns_mode is not set: rootless Podman's default maps container root to your host user; set `auto` explicitly for a private subordinate UID range.",
		run: check_no_userns,
	},
	CheckDescriptor {
		id: "secret_in_environment",
		description: "an environment variable name contains a secret-bearing segment (PASSWORD, SECRET, TOKEN, KEY) and a value is set in compose; move it to secrets:.",
		run: check_secret_in_environment,
	},
	CheckDescriptor {
		id: "port_published_on_all_interfaces",
		description: "a port is published without a host IP, so the bind falls on every host interface.",
		run: check_port_published_on_all_interfaces,
	},
	CheckDescriptor {
		id: "sensitive_bind_mount",
		description: "a bind mount exposes a sensitive host path: a container runtime socket, or /proc, /sys, /dev, /etc, /boot, /root or a runtime directory holding a socket.",
		run: check_sensitive_bind_mount,
	},
	CheckDescriptor {
		id: "unpinned_image",
		description: "image has no tag (defaults to :latest), pins to :latest, or is not anchored by a digest.",
		run: check_unpinned_image,
	},
];

#[cfg(test)]
#[path = "checks_more_tests.rs"]
mod more_tests;
#[cfg(test)]
#[path = "checks_port_exposure_tests.rs"]
mod port_exposure_tests;
#[cfg(test)]
#[path = "checks_secret_env_tests.rs"]
mod secret_env_tests;
#[cfg(test)]
#[path = "checks_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "checks_unpinned_image_tests.rs"]
mod unpinned_image_tests;
