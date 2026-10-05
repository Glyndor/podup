//! The client-address warning (#1994) as the CLI shows it.
//!
//! Under rootless Podman, ports published on a bridge network are forwarded
//! by rootlessport, so the container sees one internal address for every
//! client. `config` must say so for a service on the project network and stay
//! quiet for `network_mode: pasta`, which keeps the source; `ps` must not
//! repeat it, like the "published on every interface" warning.

use std::fs;
use std::process::Command;

const NEEDLE: &str = "so the container sees every client as one internal address";

fn podup(compose: &str, args: &[&str]) -> String {
	let dir = tempfile::tempdir().expect("tempdir");
	let path = dir.path().join("compose.yaml");
	fs::write(&path, compose).expect("write compose");
	let out = Command::new(env!("CARGO_BIN_EXE_podup"))
		.args(["-f", path.to_str().unwrap()])
		.args(args)
		.env_remove("RUST_LOG")
		// `ps` needs an engine; point it at a socket that does not exist so it
		// fails fast without touching a real one. Only stderr matters here.
		.env("PODMAN_SOCKET", "unix:///nonexistent/podup-test.sock")
		.output()
		.expect("run podup");
	String::from_utf8_lossy(&out.stderr).into_owned()
}

const ON_BRIDGE: &str = "services:\n  web:\n    image: nginx\n    ports:\n      - \"8080:80\"\n";

#[test]
fn config_warns_for_ports_on_the_project_network() {
	let stderr = podup(ON_BRIDGE, &["config", "-q"]);
	assert!(
		stderr.contains(NEEDLE) && stderr.contains("network_mode: pasta"),
		"{stderr}"
	);
}

#[test]
fn config_is_quiet_for_pasta() {
	let stderr = podup(
		"services:\n  web:\n    image: nginx\n    network_mode: \"pasta:-T,15432\"\n    ports:\n      - \"8080:80\"\n",
		&["config", "-q"],
	);
	assert!(!stderr.contains(NEEDLE), "{stderr}");
}

#[test]
fn ps_does_not_repeat_the_warning() {
	let stderr = podup(ON_BRIDGE, &["ps"]);
	assert!(!stderr.contains(NEEDLE), "{stderr}");
}

#[test]
fn quadlet_says_pasta_is_ignored_for_a_pod_member() {
	let compose =
		"x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    network_mode: \"pasta:-4\"\n";
	let warned = podup(compose, &["generate", "quadlet"]);
	assert!(
		warned.contains("is ignored inside the x-podman-pod pod"),
		"{warned}"
	);
	let quiet = podup(compose, &["--no-warn", "generate", "quadlet"]);
	assert!(
		!quiet.contains("is ignored inside the x-podman-pod pod"),
		"{quiet}"
	);
}
