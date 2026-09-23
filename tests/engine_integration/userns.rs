//! I compare user namespace effects with a control using Podman's defaults.

use super::*;

struct Identity {
	uid_map: String,
	uid: String,
	gid: String,
}

async fn identity(engine: &Engine, name: &str) -> podup::Result<Identity> {
	let uid_map = engine
		.test_exec_capture(name, vec!["cat".into(), "/proc/self/uid_map".into()])
		.await?;
	let uid = engine
		.test_exec_capture(name, vec!["id".into(), "-u".into()])
		.await?;
	let gid = engine
		.test_exec_capture(name, vec!["id".into(), "-g".into()])
		.await?;
	Ok(Identity { uid_map, uid, gid })
}

async fn compare_with_default(client: Client, tag: &str, mode: &str) -> (Identity, Identity) {
	let _guard = USERNS.lock().await;
	let unique = tempfile::Builder::new()
		.prefix("podup-userns-")
		.tempdir()
		.unwrap();
	let project = format!(
		"{}-{tag}",
		unique.path().file_name().unwrap().to_str().unwrap()
	);
	let engine = Engine::new(client, project.clone());
	let file = parse_str(&format!(
		"services:\n  control:\n    image: alpine:latest\n    command: [sleep, infinity]\n    network_mode: none\n    stop_grace_period: 0s\n  mapped:\n    image: alpine:latest\n    command: [sleep, infinity]\n    network_mode: none\n    stop_grace_period: 0s\n    userns_mode: {mode:?}\n"
	)).unwrap();
	let observed: podup::Result<_> = async {
		engine.up(&file).await?;
		let control = identity(&engine, &format!("{project}-control-1")).await?;
		let mapped = identity(&engine, &format!("{project}-mapped-1")).await?;
		Ok((control, mapped))
	}
	.await;
	let cleanup = engine.down(&file).await;
	// No formatted value here, like the assertions changed in #1611: CodeQL
	// reads anything from `down` as tainted because it removes the project's
	// secrets, and only an interpolated value is a sink.
	assert!(cleanup.is_ok(), "user namespace cleanup failed");
	observed.unwrap_or_else(|error| {
		panic!("userns_mode {mode:?} failed: {error}; auto requires enough unused subordinate UID and GID ranges in /etc/subuid and /etc/subgid; missing or exhausted ranges cannot satisfy these assertions")
	})
}

fn first_mapping(map: &str) -> [u64; 3] {
	let fields: Vec<u64> = map
		.lines()
		.next()
		.expect("uid_map must not be empty")
		.split_whitespace()
		.map(|part| part.parse().expect("uid_map must contain integers"))
		.collect();
	fields.try_into().expect("uid_map must have three columns")
}

#[tokio::test]
async fn auto_allocates_a_private_range() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let (control, mapped) = compare_with_default(client, "auto", "auto").await;
	assert_ne!(
		first_mapping(&mapped.uid_map),
		first_mapping(&control.uid_map),
		"auto must allocate a private subordinate ID range"
	);
	assert_eq!(control.uid.trim(), "0");
	assert_eq!(mapped.uid.trim(), "0");
}

#[tokio::test]
async fn auto_size_controls_the_allocated_range() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let (control, mapped) = compare_with_default(client, "size", "auto:size=2048").await;
	let mapping = first_mapping(&mapped.uid_map);
	assert_ne!(
		mapping,
		first_mapping(&control.uid_map),
		"auto:size must allocate a private subordinate ID range"
	);
	assert_eq!(mapping[0], 0);
	assert_eq!(mapping[2], 2048);
	assert_eq!(mapped.uid_map.lines().count(), 1);
	assert_eq!(control.uid.trim(), "0");
	assert_eq!(mapped.uid.trim(), "0");
}

#[tokio::test]
async fn keep_id_options_choose_the_container_identity() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let (control, mapped) = compare_with_default(client, "keep", "keep-id:uid=123,gid=456").await;
	assert_ne!(
		first_mapping(&mapped.uid_map),
		first_mapping(&control.uid_map)
	);
	assert_eq!(control.uid.trim(), "0");
	assert_eq!(mapped.uid.trim(), "123");
	assert_eq!(mapped.gid.trim(), "456");
}
