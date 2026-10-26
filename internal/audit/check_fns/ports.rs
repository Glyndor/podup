use podup::compose::types::{ComposeFile, Service};

use super::{finding, Finding};

// The audit must use the same notion of "published on every interface" as
// the parse-time port-exposure warning `up`/`config` already emit (#1835).
use podup::ports_published_on_all_interfaces_in;
// Same shared parser as the all-interfaces check: the audit's wildcard
// finding runs the same parser code-path so the two cannot disagree on
// what `0.0.0.0` or `[::]` in a `ports:` string actually means (#1881).
use podup::ports_published_on_wildcard_in;

/// One [`Finding`] per port the container publishes without a host IP, so
/// the bind falls on every host interface. The check delegates to
/// [`podup::ports_published_on_all_interfaces_in`] so its notion of
/// "published on every interface" is the exact same predicate the
/// parse-time warning in `internal/parsing/compose/diagnostics/port_exposure.rs`
/// already uses; the audit is the same opinion as `up`/`config`, not
/// a divergent second one (#1835). The predicate is scoped to the
/// container the check runs against, so an init step's own ports are
/// judged like any other container's.
///
/// Threshold (same as the diagnostic warning):
/// - Short form with no IP (`"5432:5432"`, `"8080:80/tcp"`): flagged.
/// - Short form with an explicit IP, including `0.0.0.0` and a private
///   LAN address like `192.168.1.10`: NOT flagged. An explicit bind is
///   a decision taken; flagging it would only train the reader to
///   ignore the check, the same argument the diagnostic's own comment
///   makes.
/// - Short form with 0 colons (`"80"`): NOT flagged. Container-only
///   is the short-form mirror of `expose:`, not a publish.
/// - Short form `[::1]:5432:5432`: NOT flagged. IPv6 carries its own
///   host-IP marker.
/// - Long form with `published` but no `host_ip`: flagged.
/// - Long form with `host_ip` set (any non-empty value): NOT flagged.
/// - Long form with no `published`: NOT flagged. The port is exposed,
///   not published on the host.
///
/// Port range (`published: "8080-8090"`): one finding for the mapping,
/// not one per port in the range. The label carried in the reason is
/// the range string verbatim, so an operator sees `port 8080-8090`
/// rather than twenty duplicates. Severity is the same as a
/// single-port mapping; multiplying findings across a range adds
/// noise without adding signal.
pub fn check_port_published_on_all_interfaces(
	service_name: &str,
	service: &Service,
	_file: &ComposeFile,
) -> Vec<Finding> {
	let mut out = Vec::new();
	for (_, host) in ports_published_on_all_interfaces_in(service_name, service) {
		out.push(finding(
			service_name,
			"port_published_on_all_interfaces",
			&format!(
				"port {host} is published on every interface; bind to 127.0.0.1 (or another host IP) to keep it off the network"
			),
		));
	}
	out
}

/// One [`Finding`] per port the container publishes with an explicit
/// wildcard host IP (`0.0.0.0` or `::`). The check fires only when the
/// opt-in flag `--wildcard-binds` is set (`#1881`); without it, the
/// registry entry is still listed by `audit --list-checks` (so an
/// integrator can see the option exists) and absent from the audit
/// findings (so `--strict` keeps meaning exactly what it did before).
/// The predicate is scoped to the container the check runs against, so an
/// init step's own ports are judged like any other container's.
///
/// Threshold (mirror of the all-interfaces predicate in
/// [`crate::compose::diagnostics::port_exposure::ports_published_on_all_interfaces`],
/// filtering the other way: the operator typed `0.0.0.0` or `::`):
///
/// - Short form `0.0.0.0:host:container`: flagged.
/// - Short form `[::]:host:container`: flagged.
/// - Short form with any other IP (`127.0.0.1`, `192.168.1.10`,
///   `[::1]`, `fd00::1`): not flagged. The address is a deliberate
///   decision and a stricter audit agrees.
/// - Short form with 1 colon (`"5432:5432"`, no IP): not flagged here.
///   That case is the all-interfaces check's job; if both fired on the
///   same mapping the audit would double-count the same risk.
/// - Long form with `host_ip: "0.0.0.0"` or `host_ip: "::"`: flagged.
/// - Long form with any other `host_ip` value: not flagged.
/// - Long form with no `published`: not flagged. The port is exposed,
///   not published on the host.
///
/// Port range (`published: "8080-8090"` with `host_ip: "0.0.0.0"`):
/// one finding for the mapping, not one per port in the range. Same
/// shape as the all-interfaces check; multiplying findings across a
/// range adds noise without adding signal.
pub fn check_port_published_on_wildcard(
	service_name: &str,
	service: &Service,
	_file: &ComposeFile,
) -> Vec<Finding> {
	let mut out = Vec::new();
	for (_, host) in ports_published_on_wildcard_in(service_name, service) {
		out.push(finding(
			service_name,
			"port_published_on_wildcard",
			&format!(
				"port {host} is bound to a wildcard address; bind to 127.0.0.1 (or another host IP) to keep it off the network"
			),
		));
	}
	out
}
