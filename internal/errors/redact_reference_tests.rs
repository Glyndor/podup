use super::{redact_label, redact_request_path, redact_urls_in_text, remember_reference};

#[test]
fn an_image_reference_with_credentials_loses_them_wherever_it_is_repeated() {
	for (index, (text, expected)) in [
		(
			"parsing reference \"uA1:pwB2@127.0.0.1:1/img:tag\": invalid reference format",
			"parsing reference \"127.0.0.1:1/img:tag\": invalid reference format",
		),
		(
			"uA1:pwB2@registry.example/team/img:1",
			"registry.example/team/img:1",
		),
		(
			"pull 'uA1:pw,B(2)@registry.example:5000/img:1' failed",
			"pull 'registry.example:5000/img:1' failed",
		),
		(
			"uA1:pwB2@registry.example/img@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
			"registry.example/img@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
		),
	]
	.into_iter()
	.enumerate()
	{
		assert!(redact_urls_in_text(text) == expected, "case {index}");
	}
}

#[test]
fn names_that_are_not_credentials_are_left_alone() {
	for text in [
		"alpine:3.20@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
		"registry.example:5000/app:1@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
		"mail someone@example.com about it",
		"git@host.example:org/repo.git",
		"localhost:5000/img:tag",
		"proj-web-1",
		"at 10:30 on host:8080/path",
	] {
		assert_eq!(redact_urls_in_text(text), text);
	}
}

#[test]
fn the_user_of_a_reference_is_hidden_when_a_later_message_repeats_it_alone() {
	remember_reference("uQ7xx9:pwR8xx9@registry.example/img:1");
	let shown = redact_urls_in_text("unknown transport \"uQ7xx9\" and password pwR8xx9");
	assert!(!shown.contains("uQ7xx9"), "the user came out");
	assert!(!shown.contains("pwR8xx9"), "the password came out");
}

// A credential that is known is hidden before the text is read for its URLs,
// not only after: reading `?` as the start of a query would cut the password
// of this one in two and leave the front of it, and the user, in the text.
#[test]
fn a_known_password_that_holds_a_query_mark_is_hidden_whole() {
	remember_reference("usrAB1:pa?ss1234@host.example/img");
	let shown = redact_urls_in_text("clone https://usrAB1:pa?ss1234@host.example/r.git failed");
	assert_eq!(shown, "clone https://host.example/r.git failed");
}

// What hides a known part must not end the URL it sits in: the rest of a
// longer secret that holds the part would be left behind.
#[test]
fn a_known_part_inside_a_longer_secret_does_not_leave_the_rest_behind() {
	remember_reference("usrCD2:pwX9known@host.example/img");
	let shown = redact_urls_in_text("https://other:aapwX9knownTAILSECRET@host.example/r.git");
	assert!(
		!shown.contains("TAILSECRET"),
		"the tail of the secret came out"
	);
}

#[test]
fn a_password_with_a_slash_and_a_reference_with_no_path_are_cleaned() {
	for (text, expected) in [
		("uP1xxx:pa/ss9876@reg.example/img:1", "reg.example/img:1"),
		("uP2xxx:pwS2xxx@reg.example", "reg.example"),
		("uP3xxx:pwS3xxx@reg.example:5000", "reg.example:5000"),
	] {
		assert_eq!(redact_urls_in_text(text), expected);
	}
}

#[test]
fn a_digest_is_not_user_information_wherever_the_reference_ends() {
	for text in [
		"app:1@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
		"app@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
		"reg.example:5000/team/app:1@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
	] {
		assert_eq!(redact_urls_in_text(text), text);
	}
}

// What the engine says is not trusted to teach what to hide: a flood of
// messages that look like references must not push out what the configuration
// registered.
#[test]
fn messages_from_the_engine_do_not_push_out_what_the_configuration_taught() {
	remember_reference("uTrustedUser9:pwTrusted99@registry.example/img");
	for n in 0..64 {
		let _ = redact_urls_in_text(&format!(
			"uFlood{n:03}x:pwFlood{n:03}x@registry.example/img"
		));
	}
	let shown = redact_urls_in_text("the password was pwTrusted99");
	assert!(
		!shown.contains("pwTrusted99"),
		"the registered password came out"
	);
}

#[test]
fn the_segments_of_a_request_path_are_cleaned_one_by_one() {
	let path = "/v6.2.0/libpod/images/uA1xxx%3Apw%2Fb2xxx%40127.0.0.1%3A1%2Fteam%2Fimg%3Atag/json";
	assert_eq!(
		redact_request_path(path),
		"/v6.2.0/libpod/images/127.0.0.1:1/team/img:tag/json"
	);
	let plain = "/v6.2.0/libpod/images/alpine%3A3.20/json";
	assert_eq!(redact_request_path(plain), plain);
}

// The mark of a known user stands at the start of the word, where the
// reference begins: the second reference shares the user of the first one and
// has a password of its own.
#[test]
fn a_reference_that_shares_a_known_user_still_loses_its_own_password() {
	remember_reference("usrEF3xx:pwA1xxxx@registry.example/one");
	let shown = redact_urls_in_text("pull usrEF3xx:pwB2yyyy@other.example/two failed");
	assert_eq!(shown, "pull other.example/two failed");
}

#[test]
fn a_reference_pinned_by_digest_loses_the_user_information_it_holds() {
	let digest = "@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
	for (text, expected) in [
		("uD1xxx:pa/ss9876@reg.example/img", "reg.example/img"),
		("uD2xxx:pwS2xxx@img", "img"),
		(
			"uD3xxx:pwS3xxx@reg.example:5000/team/img:1",
			"reg.example:5000/team/img:1",
		),
	] {
		assert_eq!(
			redact_urls_in_text(&format!("{text}{digest}")),
			format!("{expected}{digest}")
		);
	}
}

#[test]
fn a_known_reference_repeated_in_a_message_leaves_nothing_of_its_user_information() {
	remember_reference("usrGH4xx:pwC3xxxx@reg.example/a");
	assert_eq!(
		redact_urls_in_text("pull usrGH4xx:pwC3xxxx@reg.example/a failed"),
		"pull reg.example/a failed"
	);
}

// The second secret holds the first one: it has to be hidden whole.
#[test]
fn a_known_secret_that_holds_another_known_one_is_hidden_whole() {
	remember_reference("usrIJ5xx:pwD4xxxx@h.example/a");
	remember_reference("usrIJ5xx:pwD4xxxxTAILPART@h.example/b");
	let shown = redact_urls_in_text("the secret pwD4xxxxTAILPART was used");
	assert!(
		!shown.contains("TAILPART"),
		"the rest of the longer secret came out"
	);
}

// A blank, a quote and a bracket end a word of a message and can be in a
// password; a name and a segment of a path are one reference.
#[test]
fn a_password_with_a_blank_a_quote_and_a_bracket_is_taken_out_of_a_name() {
	assert_eq!(
		redact_label("usrKL6xx:pa ss\"w<o>rd@reg.example/img:1"),
		"reg.example/img:1"
	);
	let path = "/v6.2.0/libpod/images/usrKL7xx%3Apb%20tt%22x%3Cp%3Ese%40reg.example%2Fimg%3A1/json";
	assert_eq!(
		redact_request_path(path),
		"/v6.2.0/libpod/images/reg.example/img:1/json"
	);
}

// A name is one reference, read whole: what comes before the last `@` is the
// user information whatever it holds.
#[test]
fn a_name_is_read_whole_whatever_its_user_information_holds() {
	let digest = "@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
	for (name, expected) in [
		("u:/x@h/i", "h/i"),
		("u:123/x@h/i", "h/i"),
		("u:p@a/b@h/i", "h/i"),
		(":t@h/i", "h/i"),
		("t@h/i", "h/i"),
		("docker://u:p@h/i", "docker://h/i"),
		("docker://t@h/i", "docker://h/i"),
		("docker://u:p@a/b@h/i", "docker://h/i"),
	] {
		assert_eq!(redact_label(name), expected);
		let pinned = format!("{name}{digest}");
		assert_eq!(redact_label(&pinned), format!("{expected}{digest}"));
	}
	for name in [
		"proj-web-1",
		"h/i",
		"h:5000/i:1",
		"app:1@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
	] {
		assert_eq!(redact_label(name), name);
	}
}

// What is known is learned where the reference enters, before a message can
// repeat it, not where something happens to be written about it.
#[test]
fn a_reference_that_was_remembered_is_hidden_in_a_message_that_names_only_its_user() {
	let user = "usrMN8xx";
	assert!(redact_urls_in_text(&format!("unknown transport \"{user}\"")).contains(user));
	remember_reference(&format!("{user}:pwE5xxxx@registry.example/img"));
	assert!(!redact_urls_in_text(&format!("unknown transport \"{user}\"")).contains(user));
}
