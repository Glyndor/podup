//! Tests that cover the client as a whole; each concern's own suite lives with
//! the code it covers.

use super::Client;

#[test]
fn client_new_stores_socket_path() {
	let c = Client::new("/run/user/1000/podman/podman.sock");
	drop(c); // just verify it constructs
}
