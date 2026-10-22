use super::*;

#[test]
fn state_is_active_only_for_running_and_paused() {
	// `stop` actually transitions only a running or paused container; for any
	// other state it is a no-op that must not be reported as "stopped" (#876).
	assert!(state_is_active("running"));
	assert!(state_is_active("paused"));
	assert!(!state_is_active("created"));
	assert!(!state_is_active("exited"));
	assert!(!state_is_active("stopped"));
	assert!(!state_is_active("dead"));
	assert!(!state_is_active("configured"));
	assert!(!state_is_active(""));
}
