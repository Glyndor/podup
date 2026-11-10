use super::*;

fn opts(list: &[&str]) -> Vec<String> {
	list.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn options_without_an_owner_are_left_as_they_are() {
	let o = opts(&["size=64m", "mode=1777", "noexec"]);
	assert_eq!(translate_owner(&o, None).unwrap(), o);
	assert_eq!(translate_owner(&o, Some("1000")).unwrap(), o);
}

#[test]
fn an_owner_that_is_the_user_becomes_u() {
	// Measured: Podman 5.7.0 refuses `uid=1000` and, with `U`, gives the tmpfs
	// to the user the container runs as (1000:1000 for `user: "1000:1000"`).
	let got = translate_owner(
		&opts(&["uid=1000", "gid=1000", "mode=1777"]),
		Some("1000:1000"),
	)
	.unwrap();
	assert_eq!(got, opts(&["mode=1777", "U"]));
	let only_uid = translate_owner(&opts(&["uid=1000"]), Some("1000")).unwrap();
	assert_eq!(only_uid, opts(&["U"]));
}

#[test]
fn an_owner_without_a_user_is_refused_and_says_what_to_set() {
	let msg = translate_owner(&opts(&["uid=1009", "gid=1009"]), None).unwrap_err();
	assert!(msg.contains("user: \"1009:1009\""), "{msg}");
}

#[test]
fn a_half_owner_never_suggests_a_value_that_is_a_name_to_fill_in() {
	// `user: "1000:GID"` would be a group called GID; the part the file did not
	// give is a visibly empty placeholder.
	let msg = translate_owner(&opts(&["uid=1000"]), None).unwrap_err();
	assert!(msg.contains("user: \"1000:<gid>\""), "{msg}");
	assert!(!msg.contains(":GID"), "{msg}");
	let msg = translate_owner(&opts(&["gid=1000"]), None).unwrap_err();
	assert!(msg.contains("user: \"<uid>:1000\""), "{msg}");
}

#[test]
fn an_owner_that_is_not_the_user_is_refused() {
	for (o, user) in [
		(&["uid=1000"][..], "2000:2000"),
		(&["uid=1000", "gid=1000"][..], "1000:2000"),
		(&["uid=1000", "gid=1000"][..], "1000"), // the group of `user: 1000` is the image's to say
		(&["uid=1000"][..], "appuser"),
	] {
		let msg = translate_owner(&opts(o), Some(user)).unwrap_err();
		assert!(msg.contains("Podman has no uid/gid"), "{user}: {msg}");
	}
}
