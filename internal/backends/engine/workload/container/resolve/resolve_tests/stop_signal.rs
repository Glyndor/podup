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

/// `stop_signal` comes from the compose file. The byte after `RTMIN`/`RTMAX`
/// was split off with `split_at(1)`, which aborts the process when that byte
/// starts a multi-byte character, and the offset was added unchecked.
#[test]
fn stop_signal_with_a_multibyte_character_after_rtmin_is_an_error_not_a_panic() {
	for hostile in ["RTMIN€", "SIGRTMAX\u{e9}1", "rtmin\u{1f4a5}"] {
		let err = resolve_stop_signal(hostile).unwrap_err();
		assert!(
			err.to_string().contains("unknown stop_signal"),
			"{hostile:?}: {err}"
		);
	}
}

#[test]
fn stop_signal_offset_that_overflows_is_an_error() {
	for hostile in [
		"RTMIN+9223372036854775807",
		"RTMAX-9223372036854775807",
		"RTMAX--9223372036854775808",
	] {
		assert!(resolve_stop_signal(hostile).is_err(), "{hostile}");
	}
}
