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

/// Every line of a `uid_map` as `[inside, outside, length]`.
fn mappings(map: &str) -> Vec<[u64; 3]> {
	map.lines()
		.map(|line| {
			let fields: Vec<u64> = line
				.split_whitespace()
				.map(|part| part.parse().expect("uid_map must contain integers"))
				.collect();
			fields.try_into().expect("uid_map must have three columns")
		})
		.collect()
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
	let lines = mappings(&mapped.uid_map);
	assert_ne!(
		lines[0],
		first_mapping(&control.uid_map),
		"auto:size must allocate a private subordinate ID range"
	);
	// Podman takes the range from the free holes of the subordinate pool,
	// first fit, and splits it when no single hole is large enough: next to a
	// 1024-ID hole left by other containers, `auto:size=2048` comes back as two
	// lines of 1024 (measured on Podman 5.7.0). What the size controls is the
	// total, starting at 0 inside the container with no gap, not how many
	// lines the host side takes.
	let mut inside = 0;
	for [start, _, length] in &lines {
		assert_eq!(
			*start, inside,
			"the container side must be one run from 0: {lines:?}"
		);
		inside += length;
	}
	assert_eq!(inside, 2048, "auto:size=2048 must map 2048 IDs: {lines:?}");
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
