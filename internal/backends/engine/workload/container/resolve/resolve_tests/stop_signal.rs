use super::*;

#[test]
fn stop_signal_resolves_names_numbers_and_prefixes() {
	// Named, case-insensitive, with and without the SIG prefix.
	assert_eq!(resolve_stop_signal("SIGTERM").unwrap(), 15);
	assert_eq!(resolve_stop_signal("TERM").unwrap(), 15);
	assert_eq!(resolve_stop_signal("sigterm").unwrap(), 15);
	assert_eq!(resolve_stop_signal("SIGKILL").unwrap(), 9);
	assert_eq!(resolve_stop_signal("hup").unwrap(), 1);
	assert_eq!(resolve_stop_signal("SIGUSR1").unwrap(), 10);
	assert_eq!(resolve_stop_signal("SIGUSR2").unwrap(), 12);
	// Bare numbers pass through (optionally surrounded by whitespace).
	assert_eq!(resolve_stop_signal("15").unwrap(), 15);
	assert_eq!(resolve_stop_signal(" 9 ").unwrap(), 9);
}

#[test]
fn stop_signal_resolves_realtime_signals() {
	// Realtime signals and their offset forms, as docker-compose/Podman accept
	// them (images on s6-overlay/tini commonly use SIGRTMIN+3).
	assert_eq!(resolve_stop_signal("SIGRTMIN").unwrap(), 34);
	assert_eq!(resolve_stop_signal("SIGRTMIN+3").unwrap(), 37);
	assert_eq!(resolve_stop_signal("SIGRTMAX").unwrap(), 64);
	assert_eq!(resolve_stop_signal("SIGRTMAX-1").unwrap(), 63);
	// The SIG prefix is optional here too.
	assert_eq!(resolve_stop_signal("RTMIN+3").unwrap(), 37);
}

#[test]
fn stop_signal_rejects_out_of_range_realtime_offset() {
	// 34 + 40 = 74 is past SIGRTMAX (64), so it is rejected.
	let err = resolve_stop_signal("SIGRTMIN+40").unwrap_err();
	assert!(err.to_string().contains("SIGRTMIN+40"), "got: {err}");
	// Wrong-direction offsets are rejected too.
	assert!(resolve_stop_signal("SIGRTMIN-1").is_err());
	assert!(resolve_stop_signal("SIGRTMAX+1").is_err());
}

#[test]
fn stop_signal_unknown_name_errors() {
	let err = resolve_stop_signal("SIGNOPE").unwrap_err();
	assert!(err.to_string().contains("SIGNOPE"), "got: {err}");
}
