use super::{redact_url, redact_urls_in_text};

#[test]
fn the_user_information_and_the_query_go() {
	assert_eq!(
		redact_url("https://user:s3cr3t@host.example/org/repo.git?token=abc123"),
		"https://host.example/org/repo.git"
	);
}

#[test]
fn a_token_alone_in_the_user_information_goes_too() {
	assert_eq!(
		redact_url("https://ghp_token@host.example/repo.git"),
		"https://host.example/repo.git"
	);
}

#[test]
fn the_fragment_that_names_the_reference_stays() {
	assert_eq!(
		redact_url("https://u:p@host.example/repo.git?x=1#main:docker"),
		"https://host.example/repo.git#main:docker"
	);
}

#[test]
fn an_at_sign_in_the_path_or_fragment_is_not_user_information() {
	assert_eq!(
		redact_url("https://host.example/a@b/repo.git#v@1"),
		"https://host.example/a@b/repo.git#v@1"
	);
}

#[test]
fn a_port_and_a_url_with_nothing_to_hide_are_unchanged() {
	assert_eq!(
		redact_url("https://host.example:8443/repo.git"),
		"https://host.example:8443/repo.git"
	);
	assert_eq!(
		redact_url("git://host.example/repo.git"),
		"git://host.example/repo.git"
	);
}

#[test]
fn an_authority_with_no_path_is_handled() {
	assert_eq!(
		redact_url("https://u:p@host.example"),
		"https://host.example"
	);
	assert_eq!(
		redact_url("https://u:p@host.example?token=x"),
		"https://host.example"
	);
}

#[test]
fn the_scp_form_keeps_its_user_and_loses_a_password() {
	assert_eq!(
		redact_url("git@host.example:org/repo.git"),
		"git@host.example:org/repo.git"
	);
	assert_eq!(
		redact_url("user:pw@host.example:org/repo.git"),
		"host.example:org/repo.git"
	);
}

#[test]
fn every_url_in_a_message_is_redacted_and_the_rest_is_kept() {
	let message = "cloning \"https://u:pw@127.0.0.1:1/x.git?token=tk#main\" to \"/var/tmp/b\":\n\
		fatal: unable to access 'https://127.0.0.1:1/x.git?token=tk/': Failed to connect";
	assert_eq!(
		redact_urls_in_text(message),
		"cloning \"https://127.0.0.1:1/x.git#main\" to \"/var/tmp/b\":\n\
		fatal: unable to access 'https://127.0.0.1:1/x.git': Failed to connect"
	);
}

#[test]
fn text_without_a_url_is_unchanged() {
	for text in ["", "no such container: web", "a:b@c d?e#f", "ends with ://"] {
		assert_eq!(redact_urls_in_text(text), text);
	}
}

#[test]
fn a_url_at_the_end_of_the_text_and_two_in_a_row_are_handled() {
	assert_eq!(
		redact_urls_in_text("see https://u:p@h.example/x?q=1 and http://a:b@i.example"),
		"see https://h.example/x and http://i.example"
	);
}
