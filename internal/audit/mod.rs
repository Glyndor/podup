//! `podup audit`: render hardening findings for every service in a parsed
//! compose file. The command is a read-only view; it never contacts Podman
//! and never changes what `up` does.
//!
//! Checks live as a flat list of pure functions over the parsed
//! [`podup::compose::types::Service`]. Each check returns the list of
//! [`Finding`]s it raised; [`audit_file`] walks the file and the
//! [`render_table`]/[`render_json`] functions translate the result into the
//! `--format table|json` output the CLI asked for. `audit --list-checks` does
//! not walk a compose file at all: it enumerates the same registry the audit
//! dispatches against, so an integrator diffing the listing between releases
//! learns about every check the run path can fire.

use std::io::{self, Write};

use podup::compose::types::{ComposeFile, Service};

mod checks;

/// One detected hardening finding: a service that did something the spec
/// asked us to flag, identified by a stable snake_case id the caller can grep
/// (`podup audit --format json` is machine-readable specifically so this id
/// is stable across releases).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
	/// The name of the container the finding is about: the compose key for a
	/// service, `<service>/pre_start[i]` for an init step. Stable across
	/// reloads; the audit table sorts by this.
	pub service: String,
	/// Stable snake_case check id (`privileged`, `host_namespace`, …).
	pub check: &'static str,
	/// One-line human-readable explanation. Printed after the table row and
	/// emitted as the `reason` field in `--format json`.
	pub reason: String,
}

/// One container the audit walked: a service, or one of its init steps.
#[derive(Debug, Clone)]
pub struct AuditedContainer {
	/// The name the container is reported under: the compose key for a
	/// service, `<service>/pre_start[i]` for an init step.
	pub name: String,
	/// The container specification the checks ran against.
	pub service: Service,
}

/// What the audit walked: the file's services sorted by service name, and
/// the findings keyed by the (service, check) pairs they belong to.
#[derive(Debug, Default)]
pub struct AuditReport {
	/// Findings, in the order the checks produced them.
	pub findings: Vec<Finding>,
}

impl AuditReport {
	/// Whether at least one finding was produced.
	pub fn has_findings(&self) -> bool {
		!self.findings.is_empty()
	}

	/// Group every finding by container name, preserving the container order
	/// ([`audit_file`] visits the services in YAML order). The findings
	/// inside each group stay as refs into `self` so the renderer can pull
	/// the check id and reason without cloning.
	fn by_service<'a, 'b>(
		&'a self,
		containers: &'b [AuditedContainer],
	) -> Vec<(&'b str, Vec<&'a Finding>)> {
		let mut out = Vec::with_capacity(containers.len());
		for container in containers {
			let mine: Vec<&'a Finding> = self
				.findings
				.iter()
				.filter(|f| f.service == container.name)
				.collect();
			out.push((container.name.as_str(), mine));
		}
		out
	}
}

/// Run every check against every container in `file`, returning the
/// accumulated findings. Containers are visited in the order they appear in
/// [`ComposeFile::services`] (insertion order, which the parser preserves from
/// the YAML), and each service is followed by its init steps. A container
/// with no findings contributes nothing to the report.
///
/// `opt_in` is the slice of opt-in flag names the CLI has enabled on
/// this run (`#1881`). An entry whose registry `opt_in` is `None`
/// always runs; an entry whose `opt_in` is `Some(flag)` only runs
/// when `flag` is in the slice. Passing `&[]` reproduces the legacy
/// behaviour where every check runs unconditionally; the binary test
/// `audit_strict_exits_zero_when_clean` and the in-process tests in
/// `audit_tests.rs` rely on that to keep their findings list stable.
///
/// Pure: no I/O, no logging, no global state. The callers wire the result
/// into whichever renderer matches `--format`.
pub fn audit_file(file: &ComposeFile, opt_in: &[&'static str]) -> AuditReport {
	let mut report = AuditReport::default();
	for container in audited_containers(file) {
		let findings = checks::run_checks(&container.name, &container.service, file, opt_in);
		report.findings.extend(findings);
	}
	report
}

/// Every container a compose file declares, in declaration order: each
/// service, then the init steps of that service, then the same for each job.
///
/// An init step is a container of its own and the audit reports on
/// containers: a step that asks for `privileged: true` or mounts a host
/// path is a finding about that step, not about the service it belongs to.
/// The step is the effective one, the service's fields completed by the
/// step's, because that is the container that runs.
pub fn audited_containers(file: &ComposeFile) -> Vec<AuditedContainer> {
	let mut out = Vec::new();
	// A job is a container with the same exposure as a service: a `privileged`
	// job is as much a finding as a `privileged` service.
	let jobs = file.jobs.iter().map(|(name, job)| (name, &job.service));
	for (name, service) in file.services.iter().chain(jobs) {
		out.push(AuditedContainer {
			name: name.clone(),
			service: service.clone(),
		});
		for (step_name, step) in podup::compose::pre_start::effective_steps(name, service) {
			out.push(AuditedContainer {
				name: step_name,
				service: step,
			});
		}
	}
	out
}

/// Render `report` for `containers` in `table` form to stdout.
///
/// Layout: header `SERVICE FINDINGS`, one row per service with the check ids
/// joined by single spaces (or `-` when there are none), then one line per
/// finding with `  <service>: <check>: <reason>` indented two spaces. When
/// nothing was found, only the line `no findings` is printed, the table is
/// suppressed so a CI log without any findings is minimal.
///
/// Service names and reasons are user-controlled (compose YAML), so the
/// detail lines route them through `sanitize_cell` before printing; the
/// table itself already does that via `Table::print`. Without sanitization
/// a malicious compose file injects raw ESC bytes into the operator's
/// terminal, and the bytes survive a pipe because `println!` is just a
/// byte stream.
pub fn render_table(containers: &[AuditedContainer], report: &AuditReport) {
	let stdout = io::stdout();
	let mut out = stdout.lock();
	let _ = render_table_to(&mut out, containers, report);
}

/// [`render_table`] writing to `w`, factored out so tests can capture the
/// rendered bytes without redirecting the process's stdout. The sanitization
/// of user-controlled values happens here, not in the caller, so every sink
/// of audit output goes through the same gate.
pub fn render_table_to<W: Write>(
	w: &mut W,
	containers: &[AuditedContainer],
	report: &AuditReport,
) -> io::Result<()> {
	let mut table = podup::ui::Table::new(&["SERVICE", "FINDINGS"]);
	for (name, mine) in report.by_service(containers) {
		let findings = if mine.is_empty() {
			"-".to_string()
		} else {
			mine.iter().map(|f| f.check).collect::<Vec<_>>().join(" ")
		};
		table.push(vec![(*name).to_string(), findings]);
	}
	if report.findings.is_empty() {
		writeln!(w, "no findings")?;
		return Ok(());
	}
	table.print_to(w)?;
	for finding in &report.findings {
		// `check` is a hardcoded `'static str`, so only the user-controlled
		// `service` and `reason` need sanitizing. Going through `sanitize_cell`
		// keeps the table cell and detail line consistent: same function,
		// same rules, no drift between the two surfaces an attacker can read.
		writeln!(
			w,
			"  {}: {}: {}",
			podup::ui::sanitize_cell(&finding.service),
			finding.check,
			podup::ui::sanitize_cell(&finding.reason),
		)?;
	}
	Ok(())
}

/// Render `report` as a single JSON line to stdout. `serde_json` orders the
/// object keys alphabetically (`findings`, then per-finding `check`, `reason`,
/// `service`), which gives the stable shape CI scripts can rely on.
///
/// Empty finding lists emit `{"findings":[]}`, never `null`, so an empty
/// body is still valid JSON without the consumer having to special-case it.
pub fn render_json(report: &AuditReport) {
	let stdout = io::stdout();
	let mut out = stdout.lock();
	render_json_to(&mut out, report).expect("write to stdout");
}

/// [`render_json`] writing to `w`. JSON output is already safe from terminal
/// injection: `serde_json` escapes every control character (`\x1b` becomes
/// `\u001b`), so a malicious compose file's bytes round-trip as escape
/// sequences inside a JSON string rather than as live escapes on the
/// terminal. The sink is exposed so a test can compare the exact JSON it
/// produces against the same input.
pub fn render_json_to<W: Write>(w: &mut W, report: &AuditReport) -> io::Result<()> {
	let findings: Vec<serde_json::Value> = report
		.findings
		.iter()
		.map(|f| {
			serde_json::json!({
				"check": f.check,
				"reason": f.reason,
				"service": f.service,
			})
		})
		.collect();
	let out = serde_json::json!({ "findings": findings });
	writeln!(w, "{out}")
}

/// Render every check the audit carries as one `<id>\t<description>` line per
/// entry. The id is the same `&'static str` a finding would carry; the
/// description is the prose summary drift detection diffs between releases.
/// `--list-checks` runs without a compose file, so the listing comes straight
/// from the registry rather than from any parsed input.
pub fn render_list_checks_table() {
	let stdout = io::stdout();
	let mut out = stdout.lock();
	let _ = render_list_checks_table_to(&mut out);
}

/// [`render_list_checks_table`] writing to `w`. The id and description are
/// both `&'static str` and ASCII (the description is the one-line prose the
/// registry carries), so no sanitisation runs here; nothing the registry
/// holds is user-controlled. The `opt_in` column carries the flag name
/// for opt-in checks and `-` for always-on checks, so an integrator
/// reading the listing sees the toggle next to the id (`#1881`).
pub fn render_list_checks_table_to<W: Write>(w: &mut W) -> io::Result<()> {
	// `id<TAB>description` is the shape integrators already parse
	// (`cut -f2` for the description), so it stays exactly that for every
	// check. An opt-in check appends its enabling flag as a third column
	// at the end, where no existing reader looks.
	for check in checks::CHECK_REGISTRY {
		match check.opt_in {
			Some(flag) => writeln!(w, "{}\t{}\topt-in: {}", check.id, check.description, flag)?,
			None => writeln!(w, "{}\t{}", check.id, check.description)?,
		}
	}
	Ok(())
}

/// Render every check the audit carries as one JSON object per entry, wrapped
/// in `{"checks": [...]}`. Same key shape a consumer already learns from
/// `audit --format json` (`findings` is `[]`, `checks` is the listing), so a
/// JSON parser treats the two as siblings of the same envelope rather than
/// two unrelated shapes.
pub fn render_list_checks_json() {
	let stdout = io::stdout();
	let mut out = stdout.lock();
	render_list_checks_json_to(&mut out).expect("write to stdout");
}

/// [`render_list_checks_json`] writing to `w`. `serde_json` orders the
/// per-entry keys alphabetically (`description`, then `id`, then
/// `opt_in`), giving the stable shape a diff between releases can rely
/// on. The `opt_in` field carries the flag name for opt-in checks and
/// `null` for always-on checks; a consumer can read it without
/// branching on the absent/present distinction (`#1881`).
pub fn render_list_checks_json_to<W: Write>(w: &mut W) -> io::Result<()> {
	let entries: Vec<serde_json::Value> = checks::CHECK_REGISTRY
		.iter()
		.map(|c| {
			serde_json::json!({
				"id": c.id,
				"description": c.description,
				"opt_in": c.opt_in,
			})
		})
		.collect();
	let out = serde_json::json!({ "checks": entries });
	writeln!(w, "{out}")
}

#[cfg(test)]
#[path = "list_checks_tests.rs"]
mod list_checks_tests;
#[cfg(test)]
#[path = "pre_start_tests.rs"]
mod pre_start_tests;
#[cfg(test)]
#[path = "audit_tests.rs"]
mod tests;
