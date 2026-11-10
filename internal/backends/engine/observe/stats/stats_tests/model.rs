use super::*;

// --- the stat as the daemon sends it -------------------------------------

#[test]
fn stat_tolerates_null_network() {
	// libpod sends `"Network": null` for a container with no interfaces; it
	// must deserialize to an empty map rather than erroring.
	let json = r#"{"Name":"proj-web","CPU":1.0,"Network":null}"#;
	let stat: ContainerStat = serde_json::from_str(json).unwrap();
	assert_eq!(stat.name, "proj-web");
	assert!(stat.network.is_empty());
}

#[test]
fn stat_tolerates_missing_network() {
	let json = r#"{"Name":"proj-web"}"#;
	let stat: ContainerStat = serde_json::from_str(json).unwrap();
	assert!(stat.network.is_empty());
}
