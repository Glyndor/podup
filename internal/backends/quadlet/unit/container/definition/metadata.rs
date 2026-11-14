//! The metadata a container carries: its labels, its auto-update policy, its
//! annotations and its sysctls.

use std::path::Path;

use crate::compose::types::Service;
use crate::engine::{
	read_label_file_labels, resolve_container_labels, LabelFileIssue, LabelFileLabels,
};

use super::super::{sorted_label_pairs, Section};

/// Emit the service's own and label-file `Label=` lines, the two ownership
/// labels, and the `AutoUpdate=` line the `x-podman-autoupdate` extension maps
/// to.
pub(in super::super) fn emit_labels(
	name: &str,
	project: &str,
	service: &Service,
	base_dir: &Path,
	container: &mut Section,
	warnings: &mut Vec<String>,
) {
	let LabelFileLabels {
		labels: file_labels,
		issues,
	} = read_label_file_labels(service, base_dir);
	for issue in issues {
		match issue {
			LabelFileIssue::Unreadable { path, error } => warnings.push(format!(
				"{name}: label_file: cannot read {}: {error}",
				path.display()
			)),
			LabelFileIssue::Rejected { path, line, reason } => warnings.push(format!(
				"{name}: label_file {} line {line}: rejected ({reason}); the label is omitted",
				path.display()
			)),
		}
	}
	let labels = resolve_container_labels(service, file_labels);
	for (key, val) in sorted_label_pairs(labels) {
		container.add("Label", format!("{key}={val}"));
	}
	// Ownership labels, mirroring the live engine: tag every generated container
	// with its project and service so it is traceable/removable by label the same
	// way a running one is.
	container.add("Label", format!("podup.project={project}"));
	container.add("Label", format!("podup.service={name}"));
	// The `x-podman-autoupdate` extension. Quadlet's `AutoUpdate=` key sets
	// the `io.containers.autoupdate` label itself at daemon-reload, so the
	// `Label=` line is intentionally NOT emitted here, that would duplicate
	// it. An invalid value is warned about rather than refused: generation has
	// no error channel here, and writing an unrecognised `AutoUpdate=` would
	// make Quadlet drop the whole unit at daemon-reload, a far worse failure
	// than the key being absent. The live `up` path rejects the same value
	// outright, where it can.
	match service.podman_autoupdate() {
		Ok(Some(policy)) => container.add("AutoUpdate", policy.as_str().to_string()),
		Ok(None) => {}
		Err(e) => warnings.push(format!("{name}: {e}")),
	}
}

/// Emit one `Annotation=` line per entry in `annotations:`.
pub(in super::super) fn emit_annotations(service: &Service, container: &mut Section) {
	for ann in sorted_label_pairs(service.annotations.to_map()) {
		container.add("Annotation", format!("{}={}", ann.0, ann.1));
	}
}

/// Emit one `Sysctl=` line per entry in `sysctls:`.
pub(in super::super) fn emit_sysctls(service: &Service, container: &mut Section) {
	for (key, val) in sorted_label_pairs(service.sysctls.to_map()) {
		container.add("Sysctl", format!("{key}={val}"));
	}
}
