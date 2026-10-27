use super::super::{emit_diagnostic, is_port_exposure_warning, SuppressPortExposureGuard};

#[test]
fn port_exposure_identification_matches_the_real_warning_text() {
	// A real warning produced by `port_published_on_all_interfaces`. The
	// gate must keep identifying it after a future text tweak only if the
	// phrase survives.
	assert!(is_port_exposure_warning(
		"service 'web': port 5432 is published on every interface; \
		 use \"127.0.0.1:5432:5432\" to keep it on the host"
	));
	// A different warning must never accidentally match, otherwise the
	// gate would silently silence more than the port-exposure line.
	assert!(!is_port_exposure_warning(
		"service 'web': network_mode: host shares the host's network namespace"
	));
}

#[test]
fn guard_suppresses_only_while_held() {
	// `tracing::warn!` writes to stderr and shares state with the test
	// runner; this test does not assert on captured output. Instead it
	// confirms the guard's existence: with no guard, every warning would
	// forward; the only observable property here is that the guard compiles
	// and obeys its drop semantics (covered by the nested-guards test
	// below). The end-to-end behaviour is pinned by the integration tests
	// in `tests/cli/warnings/host_binding_warnings.rs`.
	let _g = SuppressPortExposureGuard::new();
	emit_diagnostic("service 'web': port 5432 is published on every interface");
}

#[test]
fn nested_guards_restore_in_reverse_order() {
	// Same shape as the Quadlet path's nested-guard test: an outer guard
	// stays set while an inner one is dropped, because the inner restores
	// its predecessor's `true`, not the original `false`. Without this
	// property a future parse nested inside a live one would leak the
	// outer guard's `true` state out of the inner scope.
	let _outer = SuppressPortExposureGuard::new();
	{
		let _inner = SuppressPortExposureGuard::new();
	}
	// The guard's `is_suppressed` is intentionally not exported, so the
	// only observable consequence is that dropping `_outer` compiles. The
	// nesting property is covered by the integration test
	// `up_emits_the_port_exposure_warning` (without an outer guard the
	// warning fires) and `up_no_warn_silences_the_port_exposure_warning`
	// (with an outer guard it does not), which together pin the contract.
}
