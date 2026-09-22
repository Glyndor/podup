//! #1866: every image the build produced carries `podup.project`, including
//! the intermediate stage images `labels=` would otherwise leave unlabelled.
//!
//! Lives apart from the rest of `build_images.rs` so that file stays under
//! the 500-line hard limit. The drop guard and the three label-inspection
//! helpers are co-located with the test because no other test uses them.
use super::*;

/// Drop guard that reaps every image carrying a chosen label filter when
/// the test ends, so a panic in the middle of the assertions still reaps
/// the build's intermediate stage images. The intermediate stage images
/// are untagged, so removing them by tag would leave them on disk; the
/// label is the only stable handle on the build's full output.
///
/// `socket` is the URL of the Podman socket the engine talked to. The
/// default CLI storage root is often a tmpfs separate from the socket's
/// storage, so plain `podman rmi` reaps nothing when the build wrote to
/// the socket; `--url` forwards to the socket the build used.
pub struct TestImages {
	filter: Vec<(&'static str, String)>,
	socket: String,
}

impl TestImages {
	/// One labelled build's worth of images, reaped by `podup.project` and
	/// `podup.service`. `project` and `service` are kept as `String`s so
	/// `Drop` does not need a lifetime on the test body.
	pub fn new(project: &str, service: &str, socket: &str) -> Self {
		Self {
			filter: vec![
				("podup.project", project.to_string()),
				("podup.service", service.to_string()),
			],
			socket: socket.to_string(),
		}
	}
}

impl Drop for TestImages {
	fn drop(&mut self) {
		// `podman rmi` does not accept `--filter` (the equivalent of
		// `podman images --filter`). Listing the ids first and then
		// forwarding them to `rmi -f` is the only shape that respects the
		// label filter; the previous `--filter=label=...` on `rmi` was
		// rejected as an unknown flag and silently swallowed by `let _ =`,
		// so every run of the live test left every image it built on disk.
		//
		// Both steps print the command and its stderr with `eprintln!` on a
		// non-zero exit so a failed reap is visible in the test output.
		let mut list_args: Vec<String> = vec![
			"--url".into(),
			self.socket.clone(),
			"images".into(),
			"-a".into(),
			"-q".into(),
		];
		for (k, v) in &self.filter {
			list_args.push("--filter".into());
			list_args.push(format!("label={k}={v}"));
		}
		let list_out = std::process::Command::new("podman")
			.args(&list_args)
			.output();
		let ids: Vec<String> = match list_out {
			Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
				.lines()
				.map(str::trim)
				.filter(|s| !s.is_empty())
				.map(str::to_string)
				.collect(),
			Ok(out) => {
				eprintln!(
					"TestImages::drop: `podman {}` exited {}: {}",
					list_args.join(" "),
					out.status,
					String::from_utf8_lossy(&out.stderr),
				);
				Vec::new()
			}
			Err(e) => {
				eprintln!("TestImages::drop: failed to spawn `podman`: {e}");
				Vec::new()
			}
		};
		if ids.is_empty() {
			return;
		}
		let mut rmi_args: Vec<String> = vec![
			"--url".into(),
			self.socket.clone(),
			"rmi".into(),
			"-f".into(),
		];
		rmi_args.extend(ids.iter().cloned());
		let rmi_out = std::process::Command::new("podman")
			.args(&rmi_args)
			.output();
		match rmi_out {
			Ok(out) if out.status.success() => {}
			Ok(out) => eprintln!(
				"TestImages::drop: `podman {}` exited {}: {}",
				rmi_args.join(" "),
				out.status,
				String::from_utf8_lossy(&out.stderr),
			),
			Err(e) => eprintln!("TestImages::drop: failed to spawn `podman rmi`: {e}"),
		}
	}
}

/// Locate the Podman socket the engine talks to. The CLI's own storage root
/// is often different from the socket's (a fresh CLI invocation on Linux
/// resolves to a tmpfs path the socket does not share), so plain `podman
/// images` queries the wrong store on most setups. The CLI's `--url` flag
/// forwards the request to the socket instead, and reading `info` from there
/// is what produces the storage root the test's build wrote to.
///
/// Returns `None` when no candidate socket exists; the live tests skip on
/// that path.
fn podman_socket_url() -> Option<String> {
	for path in [
		format!("/run/user/{}/podman/podman.sock", unsafe { libc::getuid() }),
		"/run/podman/podman.sock".to_string(),
	] {
		if std::path::Path::new(&path).exists() {
			return Some(format!("unix://{path}"));
		}
	}
	None
}

/// Every image id currently on the socket, full-length. The diff of two
/// snapshots is what the live test uses to learn which ids its build
/// produced.
fn snapshot_image_ids(socket: &str) -> std::collections::HashSet<String> {
	let out = std::process::Command::new("podman")
		.args(["--url", socket, "images", "-a", "-q", "--no-trunc"])
		.output()
		.expect("podman images");
	if !out.status.success() {
		panic!(
			"`podman --url {socket} images -a -q --no-trunc` exited {}: {}",
			out.status,
			String::from_utf8_lossy(&out.stderr),
		);
	}
	String::from_utf8_lossy(&out.stdout)
		.lines()
		.map(str::trim)
		.filter(|s| !s.is_empty())
		.map(str::to_string)
		.collect()
}

/// Read a single label out of an image's config. Empty string when the
/// label is absent; that is the regression the live test is meant to fail
/// on, so absent and present-but-blank are not collapsed together.
fn read_label(socket: &str, id: &str, key: &str) -> String {
	let format = format!("{{{{ index .Config.Labels \"{key}\" }}}}");
	let out = std::process::Command::new("podman")
		.args(["--url", socket, "inspect", id, "--format", &format])
		.output()
		.expect("podman inspect");
	if !out.status.success() {
		panic!(
			"`podman inspect {id}` exited {}: {}",
			out.status,
			String::from_utf8_lossy(&out.stderr),
		);
	}
	String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A multi-stage build must label every image it produces, not only the
/// final one. Without `layerLabel=`, only the final image carries
/// `podup.project`; the intermediate stage images are unlabelled and any
/// `podman image prune --filter label=podup.project=<p>` matches the lot of
/// them, including another project's intermediates left over from a sibling
/// `up`.
#[tokio::test]
async fn build_labels_every_image_including_intermediate_stages() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let Some(socket) = podman_socket_url() else {
		return;
	};
	let dir = tempfile::tempdir().unwrap();
	let project = proj("layer");
	let service = "app";
	let engine = Engine::with_base_dir(client, project.clone(), dir.path().to_path_buf());
	// Drop guard: a multi-stage build leaves intermediate stage images
	// untagged, so the only way to reap them is the same label filter the
	// test is asserting on. `podup.service` is also in the filter so a
	// project that happens to share the project name with a sibling test
	// does not get its images reaped by another test's drop.
	let _images = TestImages::new(&project, service, &socket);

	// Two stages: `base` and `final`. `--no-cache` is required to force a
	// build of the `base` stage: a cached base is not a new image, and a
	// test that let the cache short-circuit it would pass for the wrong
	// reason (one labelled image instead of three).
	fs::write(
		dir.path().join("Dockerfile"),
		b"FROM alpine:latest AS base\nRUN echo base-stage > /stage\n\
		  FROM base AS final\nRUN echo final-stage > /stage\n",
	)
	.unwrap();
	let yaml =
		"services:\n  app:\n    build:\n      context: .\n    command: [\"sleep\", \"infinity\"]\n";
	let file = parse_str(yaml).unwrap();

	// Snapshot every image id on the socket BEFORE the build, then again
	// after, and take the set difference. Filtering by the build's own
	// label inside the loop would have been tautological: every row would
	// pass by construction, and a regression that labelled two of three
	// new images would still pass. The pre-build snapshot is also the
	// only thing that distinguishes "the build produced these images" from
	// "these images were already on disk when the test started".
	let before = snapshot_image_ids(&socket);

	engine
		.build_all_with_options(
			&file,
			&[],
			&podup::BuildOptions::new(true, false, Vec::new(), false),
		)
		.await
		.expect("a multi-stage build must succeed");

	let after = snapshot_image_ids(&socket);
	let new_ids: std::collections::BTreeSet<String> = after.difference(&before).cloned().collect();

	// The diff is exposed to other tests building on the same socket at
	// the same time. Their images would carry a different `podup.service`
	// (or no `podup.project` at all), and asserting "every new id carries
	// `podup.project=<self>`" against the unfiltered set would fail on
	// those, not on a podup regression. Restrict the set to images that
	// could plausibly be from THIS build: `podup.service` matches AND
	// `podup.project` is either this project or absent. "Or absent" is
	// deliberate: the very bug this test catches is `podup.project` not
	// being applied; excluding those would silently let the regression
	// through. They stay in the set so the assertion below fails on them.
	let ours: Vec<(String, String, String)> = new_ids
		.iter()
		.filter_map(|id| {
			let svc = read_label(&socket, id, "podup.service");
			let proj = read_label(&socket, id, "podup.project");
			if svc == service && (proj == project || proj.is_empty()) {
				Some((id.clone(), svc, proj))
			} else {
				None
			}
		})
		.collect();

	assert!(
		!ours.is_empty(),
		"the build produced no new images matching `podup.service={service}` \
		 (new ids on socket: {new_ids:?})",
	);
	assert!(
		ours.len() >= 2,
		"a two-stage build with --no-cache should leave at least two new images on disk: {ours:?}",
	);
	for (id, _svc, label) in &ours {
		assert_eq!(
			label, &project,
			"every image the build produced must carry `podup.project={project}`, \
			 image {id} does not (label={label:?})",
		);
	}
}
