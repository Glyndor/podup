//! Which Podman endpoints podup will talk to at all.
//!
//! podup speaks to a local unix socket and nothing else. A remote scheme must
//! be refused before any connection is tried, and the refusal must not repeat
//! the credentials that were typed into the URL.

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use super::run::{assert_refused, podup_raw};
use super::server;

const BUDGET: Duration = Duration::from_secs(20);
const SECRET: &str = "s3cr3t-pa55";

/// 10.255.255.1 is not routable here: a client that tried it would sit in a
/// connect timeout for tens of seconds, far past this budget.
fn remote_urls() -> Vec<String> {
	["tcp", "ssh", "http", "https", "fd"]
		.iter()
		.map(|scheme| format!("{scheme}://admin:{SECRET}@10.255.255.1:2375/run/podman/podman.sock"))
		.collect()
}

#[test]
fn a_remote_scheme_in_the_socket_flag_is_refused_without_connecting() {
	for url in remote_urls() {
		let home = tempfile::tempdir().unwrap();
		let o = podup_raw(Some(&url), &[], &home, &["ps"], BUDGET);
		assert_refused(&url, &o);
		assert!(
			o.took < Duration::from_secs(5),
			"{url}: tried the network ({:?})",
			o.took
		);
		assert!(o.text.contains("not supported"), "{url}: {}", o.text);
		assert!(
			!o.text.contains(SECRET),
			"{url}: the password was printed:\n{}",
			o.text
		);
	}
}

#[test]
fn a_remote_scheme_in_docker_host_is_refused_without_connecting() {
	for url in remote_urls() {
		let home = tempfile::tempdir().unwrap();
		let o = podup_raw(None, &[("DOCKER_HOST", &url)], &home, &["ps"], BUDGET);
		assert_refused(&url, &o);
		assert!(
			o.took < Duration::from_secs(5),
			"{url}: tried the network ({:?})",
			o.took
		);
		assert!(o.text.contains("not supported"), "{url}: {}", o.text);
		assert!(
			!o.text.contains(SECRET),
			"{url}: the password was printed:\n{}",
			o.text
		);
	}
}

#[test]
fn a_remote_scheme_in_podman_socket_is_refused_without_connecting() {
	let home = tempfile::tempdir().unwrap();
	let url = &remote_urls()[0];
	let o = podup_raw(None, &[("PODMAN_SOCKET", url)], &home, &["ps"], BUDGET);
	assert_refused("PODMAN_SOCKET=tcp://", &o);
	assert!(o.text.contains("not supported"), "{}", o.text);
	assert!(!o.text.contains(SECRET), "{}", o.text);
}

#[test]
fn a_socket_another_account_owns_is_reported_not_retried() {
	// Mode 000 on the socket file stands in for a socket the caller may not
	// open, the way a socket owned by another user looks from here.
	let fake = server::start(|s, _| server::ping_ok(s));
	std::fs::set_permissions(&fake.sock, std::fs::Permissions::from_mode(0o000)).unwrap();
	let home = tempfile::tempdir().unwrap();
	let socket = format!("unix://{}", fake.sock.display());
	let o = podup_raw(Some(&socket), &[], &home, &["ps"], BUDGET);
	assert_refused("unreadable socket", &o);
	assert!(o.text.contains("cannot be opened"), "{}", o.text);
	assert!(o.took < Duration::from_secs(5), "{:?}", o.took);
}

#[test]
fn a_path_that_is_not_a_socket_is_refused_quickly() {
	let home = tempfile::tempdir().unwrap();
	let file = home.path().join("not-a-socket");
	std::fs::write(&file, b"hello").unwrap();
	for target in [
		file.display().to_string(),
		home.path().display().to_string(),
	] {
		let socket = format!("unix://{target}");
		let o = podup_raw(Some(&socket), &[], &home, &["ps"], BUDGET);
		assert_refused(&target, &o);
		assert!(o.took < Duration::from_secs(5), "{target}: {:?}", o.took);
	}
}
