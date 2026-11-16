//! Each refusal is separately selectable; every command tests both flag positions.

use super::fixture::{Fixture, COMPOSE, MISSING_BIND};

fn positions<'a>(args: &[&'a str]) -> [Vec<&'a str>; 2] {
	let mut before = args.to_vec();
	before.insert(0, "--dry-run");
	let mut after = args.to_vec();
	// Before run/exec's trailing command arguments, which belong to the container.
	after.insert(1, "--dry-run");
	[before, after]
}

fn refusal(args: &[&str], compose: &str) {
	for invocation in positions(args) {
		let fixture = Fixture::new(compose);
		let (outcome, _) = fixture.exercise(&invocation);
		let text = outcome.text();
		assert_ne!(
			outcome.code,
			Some(0),
			"{invocation:?}: silently accepted\n{text}"
		);
		assert!(
			text.contains("--dry-run"),
			"{invocation:?}: missing flag in refusal\n{text}"
		);
		assert!(
			text.split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
				.any(|s| s == args[0]),
			"{invocation:?}: missing command in refusal\n{text}"
		);
		assert!(
			!text.contains("unexpected argument") && !text.contains("unrecognized subcommand"),
			"{invocation:?}: parser error instead of dry-run refusal\n{text}"
		);
		assert!(
			!fixture.root.path().join("x").exists(),
			"{invocation:?}: created ../x"
		);
	}
}

macro_rules! refuses {
	($($name:ident: [$($arg:literal),+];)+) => {$(
		#[test]
		fn $name() { refusal(&[$($arg),+], COMPOSE); }
	)+};
}

refuses! {
	up: ["up"];
	down: ["down"];
	down_volumes: ["down", "-v"];
	down_orphans: ["down", "--remove-orphans"];
	create: ["create"];
	run: ["run", "web", "true"];
	start: ["start"];
	stop: ["stop"];
	restart: ["restart"];
	rm: ["rm"];
	kill: ["kill"];
	pause: ["pause"];
	unpause: ["unpause"];
	scale: ["scale", "web=2"];
	build: ["build"];
	pull: ["pull"];
	push: ["push"];
	cp: ["cp", "web:/data/file", "./copied"];
	exec: ["exec", "web", "true"];
	commit: ["commit", "web", "hostile-snapshot:test"];
	watch: ["watch"];
	attach: ["attach", "web"];
}

fn read_only(command: &str, marker: &str) {
	for invocation in positions(&[command]) {
		let (outcome, requests) = Fixture::new(COMPOSE).exercise(&invocation);
		assert_eq!(outcome.code, Some(0), "{invocation:?}: {}", outcome.text());
		assert!(
			outcome.stdout.contains(marker),
			"{invocation:?}: missing {marker:?}\n{}",
			outcome.text()
		);
		if matches!(command, "ps" | "ls" | "images") {
			assert!(
				requests.iter().any(|r| !r.target.contains("_ping")),
				"{invocation:?}: skipped the actual query: {requests:?}"
			);
		}
	}
}

macro_rules! allows {
	($($name:ident: $command:literal => $marker:expr;)+) => {$(
		#[test]
		fn $name() { read_only($command, $marker); }
	)+};
}

allows! {
	ps: "ps" => "hostile-web-1";
	ls: "ls" => "hostile";
	images: "images" => "alpine";
	volumes: "volumes" => "hostile_data";
	config: "config" => "services:";
	version: "version" => env!("CARGO_PKG_VERSION");
	audit: "audit" => "web";
}

#[test]
fn autostart_install_still_prints_the_unit() {
	let mut invocations = positions(&["autostart", "install"]).to_vec();
	invocations.push(vec!["autostart", "install", "--dry-run"]);
	for invocation in invocations {
		let (outcome, _) = Fixture::new(COMPOSE).exercise(&invocation);
		assert_eq!(outcome.code, Some(0), "{invocation:?}: {}", outcome.text());
		for marker in ["[Unit]", "[Service]", "ExecStart=", "hostile"] {
			assert!(
				outcome.stdout.contains(marker),
				"{invocation:?}: missing {marker}\n{}",
				outcome.text()
			);
		}
	}
}

#[test]
fn up_does_not_create_a_missing_bind_outside_the_project() {
	refusal(&["up"], MISSING_BIND);
}
