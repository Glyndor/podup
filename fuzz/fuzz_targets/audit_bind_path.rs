#![no_main]

//! Fuzz the path handling behind `podup audit`'s `sensitive_bind_mount` check:
//! the host path a compose file names is folded (`.`, `..`, repeated slashes)
//! and judged against the sensitive list. It must never panic, folding twice
//! must change nothing, and folding may only add findings: a path that is
//! sensitive as written, or as the shortest spelling of the path as written,
//! must be reported.
//!
//! The audit lives in the `podup` binary, which this crate cannot link, so the
//! real `internal/audit/check_sensitive_bind.rs` is compiled in here with the
//! two items it borrows from its parent supplied by `support/audit`.

use libfuzzer_sys::fuzz_target;
use podup::compose::types::{ComposeFile, Service, VolumeMount, VolumeType};

#[path = "../support/audit/mod.rs"]
mod audit;

use audit::checks::check_sensitive_bind::{
	check_sensitive_bind_mount, has_parent_component, normalize_host_path, normalize_slashes,
	sensitive_bind_reason,
};

fn findings(source: &str, read_only: bool) -> Vec<audit::Finding> {
	let mut service = Service::default();
	service.volumes = vec![VolumeMount::Long {
		volume_type: VolumeType::Bind,
		source: Some(source.to_string()),
		target: "/t".to_string(),
		read_only: Some(read_only),
		bind: None,
		volume: None,
		tmpfs: None,
		consistency: None,
	}];
	check_sensitive_bind_mount("svc", &service, &ComposeFile::default())
}

fuzz_target!(|data: &[u8]| {
	let Ok(text) = std::str::from_utf8(data) else {
		return;
	};
	// The check only looks at absolute paths; make every input one so the
	// interesting part is not spent on the prefix test.
	let source = if text.starts_with('/') {
		text.to_string()
	} else {
		format!("/{text}")
	};
	let source = source.as_str();

	let folded = normalize_host_path(source);
	assert!(folded.starts_with('/'), "{source:?} folded to {folded:?}");
	assert_eq!(
		normalize_host_path(&folded),
		folded,
		"folding {source:?} twice differs from folding it once"
	);
	assert!(
		!folded.contains("//") && !folded.split('/').any(|part| part == "." || part == ".."),
		"{source:?} folded to {folded:?}, which still has a `.`, a `..` or a double slash"
	);
	let written = normalize_slashes(source);
	assert_eq!(
		normalize_slashes(&written),
		written,
		"slash folding of {source:?}"
	);

	for read_only in [false, true] {
		let mode = if read_only { "read-only " } else { "" };
		let found = findings(source, read_only);
		assert!(
			found.len() <= 1,
			"{} findings for one mount of {source:?}",
			found.len()
		);
		assert!(found.iter().all(|f| f.check == "sensitive_bind_mount"));

		// Each spelling that is sensitive on its own must be reported, so
		// folding can add a finding but never take one away.
		for (label, spelling) in [("raw", source), ("written", &written), ("folded", &folded)] {
			if let Some(reason) = sensitive_bind_reason(spelling, mode) {
				let Some(first) = found.first() else {
					panic!("{label} spelling of {source:?} is sensitive ({reason}) but nothing was reported");
				};
				if label == "written" {
					assert_eq!(first.reason, reason, "reason for {source:?}");
				}
			}
		}
		// A path with `..` that is not sensitive either way still gets the
		// softer finding; anything else stays silent.
		let sensitive = sensitive_bind_reason(&written, mode).is_some()
			|| sensitive_bind_reason(&folded, mode).is_some();
		assert_eq!(
			found.is_empty(),
			!sensitive && !has_parent_component(source),
			"{source:?}: findings {found:?}"
		);
	}
});
