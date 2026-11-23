//! What a service's network attachment carries into Podman: the driver options
//! under the name libpod reads, and the aliases a `links:` entry gives its
//! target.

use super::*;

/// Every link of the host network the rootless network namespace holds, or the
/// host's own when Podman runs as root. `None` when neither can be listed.
fn host_side_links() -> Option<String> {
	let listings = [
		vec!["podman", "unshare", "--rootless-netns", "ip", "-o", "link"],
		vec!["ip", "-o", "link"],
	];
	for argv in listings {
		let Ok(out) = std::process::Command::new(argv[0])
			.args(&argv[1..])
			.output()
		else {
			continue;
		};
		if out.status.success() {
			return Some(String::from_utf8_lossy(&out.stdout).into_owned());
		}
	}
	None
}

/// `driver_opts` of an attachment went out as `driver_opts`, a key libpod's
/// decoder accepts and drops, so no option ever reached the network backend.
/// `host_interface_name` is the one option whose effect can be read from the
/// host: the veth takes the name the compose file gave.
#[cfg(feature = "test-helpers")]
#[tokio::test]
async fn attachment_driver_opts_reach_the_network_backend() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("attopts");
	let engine = Engine::new(client, proj.clone());
	let veth = format!("pv{}", std::process::id());
	let file = parse_str(&format!(
		"services:\n  web:\n    image: busybox:latest\n    command: [\"sleep\", \"infinity\"]\n    networks:\n      appnet:\n        driver_opts:\n          host_interface_name: {veth}\nnetworks:\n  appnet:\n"
	))
	.unwrap();

	engine.up(&file).await.unwrap();
	let links = host_side_links();
	engine.down(&file).await.unwrap();

	let Some(links) = links else {
		assert!(
			std::env::var_os("PODUP_REQUIRE_PODMAN").is_none(),
			"PODUP_REQUIRE_PODMAN is set but the host network links cannot be listed"
		);
		eprintln!("skipping: cannot list the host network links here");
		return;
	};
	assert!(
		links.contains(&veth),
		"the veth was not named {veth}: the attachment option did not reach netavark\n{links}"
	);
}

/// `links: [db:database]` has no field in libpod, so the alias goes on the
/// target's attachment. Both layers are read: the alias Podman holds, and the
/// name actually resolving from the linking container.
#[cfg(feature = "test-helpers")]
#[tokio::test]
async fn a_link_alias_resolves_to_the_linked_service() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("linkalias");
	let engine = Engine::new(client, proj.clone());
	let db = format!("{proj}-db-1");
	let web = format!("{proj}-web-1");
	let dir = tempfile::tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  db:\n    image: busybox:latest\n    command: [\"sh\", \"-c\", \"mkdir -p /www; echo ok > /www/index.html; exec httpd -f -p 80 -h /www\"]\n  web:\n    image: busybox:latest\n    command: [\"sleep\", \"infinity\"]\n    links:\n      - db:database\n",
	)
	.unwrap();
	let file = parse_files_with_env_files(&[compose], &[]).unwrap();

	// The same probe `dns_resolution.rs` uses for the service name: `wget`
	// resolves through the C library, retried while the server comes up and the
	// network's DNS learns the name.
	let fetch = |name: &str| {
		vec![
			"sh".to_string(),
			"-c".to_string(),
			format!(
				"for i in $(seq 1 30); do wget -q -O - http://{name}:80/ && exit 0; sleep 0.3; done; exit 1"
			),
		]
	};

	engine.up(&file).await.unwrap();
	let aliases = engine.test_container_aliases(&db).await;
	let by_alias = engine.test_exec_capture(&web, fetch("database")).await;
	// Asked only when the alias did not answer, and before `down` removes the
	// containers: does the runtime's DNS answer for the container's own name?
	let by_own_name = match &by_alias {
		Ok(out) if out.contains("ok") => None,
		_ => Some(engine.test_exec_capture(&web, fetch(&db)).await),
	};
	engine.down(&file).await.unwrap();

	let aliases = aliases.expect("could not read the db network aliases");
	assert!(
		aliases.iter().any(|a| a == "database"),
		"the link alias was not registered on db: {aliases:?}"
	);
	let by_alias = by_alias.expect("exec in web failed");
	if by_alias.contains("ok") {
		return;
	}
	// The alias is registered and did not answer. If the runtime's DNS does not
	// answer for the container's own name either, it cannot say anything about
	// the alias: a skip here, and a failure where Podman is guaranteed
	// (`dns_resolution.rs` explains the same split).
	let own_name_answers = by_own_name
		.and_then(Result::ok)
		.is_some_and(|out| out.contains("ok"));
	if !own_name_answers {
		assert!(
			std::env::var_os("PODUP_REQUIRE_PODMAN").is_none(),
			"the alias is registered, but the container runtime's DNS does not answer \
			 for the container's own name either: {by_alias:?}"
		);
		eprintln!("skipping: the container runtime's DNS is not answering");
		return;
	}
	panic!("the alias `database` is registered and the runtime's DNS answers for the container's own name, but `database` did not resolve: {by_alias:?}");
}
