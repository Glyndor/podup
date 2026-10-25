use super::*;

fn reasons(spec: &str) -> Vec<String> {
	let yaml =
		format!("services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - \"{spec}\"\n");
	report_for(&yaml)
		.iter()
		.filter(|f| f.check == "sensitive_bind_mount")
		.map(|f| f.reason.clone())
		.collect()
}

fn fires(spec: &str) -> bool {
	!reasons(spec).is_empty()
}

fn is_soft(reason: &str) -> bool {
	reason.contains("contains '..'") && reason.contains("cannot tell")
}

#[test]
fn dot_and_dot_dot_components_are_judged_in_their_folded_form() {
	for spec in [
		"/tmp/../etc:/m",
		"/./etc:/m",
		"/etc/.:/m",
		"/etc/../etc:/m",
		"/usr/../root:/m",
		"/home/../proc:/m",
		"/a/b/../../sys/./x:/m",
		"/var/run/../run/docker.sock:/s",
		"/run/./user/1000/../1000/podman/podman.sock:/s",
		"/..:/m",
		"/../..:/m",
		"//tmp//..//etc/:/m",
	] {
		let found = reasons(spec);
		assert_eq!(found.len(), 1, "{spec}: {found:?}");
		assert!(
			!is_soft(&found[0]),
			"{spec} has a sensitive target: {found:?}"
		);
	}
}

#[test]
fn nothing_that_fired_on_the_written_path_stops_firing() {
	// Each of these matched the list as written before dot segments were
	// folded. The kernel resolves `..` after following links, so the written
	// form must keep deciding even when the folded form lands elsewhere.
	for spec in [
		"/etc:/m",
		"/etc/:/m",
		"/etc/ssh:/m",
		"//etc:/m",
		"/:/m",
		"//:/m",
		"/.:/m",
		"/proc/1/root:/m",
		"/etc/../srv:/m",
		"/etc/x/..:/m",
		"/proc/../home/user:/m",
		"/root/../srv:/m",
		"/var/run/podman/../x:/m",
		"/run/docker/../data:/m",
		"/dev/../tmp:/m",
		"/var/run/docker.sock:/s",
		"/run/podman/podman.sock:/s",
		"/run/user/1000/podman/podman.sock:/s",
		"/boot/..:/m",
		"/sys/./x:/m",
	] {
		let found = reasons(spec);
		assert!(!found.is_empty(), "{spec} stopped firing");
		assert!(
			!is_soft(&found[0]),
			"{spec} lost its specific reason: {found:?}"
		);
	}
}

#[test]
fn a_dot_dot_that_matches_nothing_gets_the_softer_finding() {
	for spec in [
		"/srv/app/../data:/m",
		"/var/lib/../app:/m",
		"/home/u/../v:/m",
		"/tmp/x/..:/m",
	] {
		let found = reasons(spec);
		assert_eq!(found.len(), 1, "{spec}: {found:?}");
		assert!(is_soft(&found[0]), "{spec}: {found:?}");
	}
}

#[test]
fn a_plain_dot_or_clean_path_is_silent() {
	for spec in [
		"/srv/./app:/m",
		"/tmp/./x:/m",
		"/srv/app/data:/m",
		"/home/u/.config:/m",
	] {
		assert!(!fires(spec), "{spec} must stay silent: {:?}", reasons(spec));
	}
}

#[test]
fn the_long_form_follows_the_same_rules() {
	let long = |source: &str| {
		format!("services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - type: bind\n        source: {source}\n        target: /m\n")
	};
	let hits = |source: &str| {
		report_for(&long(source))
			.iter()
			.filter(|f| f.check == "sensitive_bind_mount")
			.map(|f| f.reason.clone())
			.collect::<Vec<_>>()
	};
	let folded = hits("/tmp/../etc");
	assert_eq!(folded.len(), 1);
	assert!(!is_soft(&folded[0]));
	let written = hits("/etc/../srv");
	assert_eq!(written.len(), 1);
	assert!(!is_soft(&written[0]));
	let soft = hits("/srv/../data");
	assert_eq!(soft.len(), 1);
	assert!(is_soft(&soft[0]));
}
