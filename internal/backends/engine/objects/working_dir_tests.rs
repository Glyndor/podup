//! The project directory is recorded on what podup creates, and a project
//! used from another directory is reported.

use std::sync::{Arc, Mutex};

use crate::engine::fake_podman;
use crate::engine::Engine;

/// The log lines of whatever runs on this thread until the guard is dropped.
struct Logs {
	bytes: Arc<Mutex<Vec<u8>>>,
	_guard: tracing::subscriber::DefaultGuard,
}

struct Sink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
	fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
		self.0.lock().unwrap().extend_from_slice(bytes);
		Ok(bytes.len())
	}
	fn flush(&mut self) -> std::io::Result<()> {
		Ok(())
	}
}

impl Logs {
	fn start() -> Self {
		let bytes = Arc::new(Mutex::new(Vec::new()));
		let writer = bytes.clone();
		let subscriber = tracing_subscriber::fmt()
			.without_time()
			.with_ansi(false)
			.with_writer(move || Sink(writer.clone()))
			.finish();
		Self {
			bytes,
			_guard: tracing::subscriber::set_default(subscriber),
		}
	}

	fn text(&self) -> String {
		String::from_utf8(self.bytes.lock().unwrap().clone()).unwrap()
	}
}

/// The warning `warn_if_project_used_elsewhere` gives when the project's one
/// network carries `labels`.
async fn warning_for(labels: &'static str, dir: &std::path::Path) -> String {
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/networks/json") {
			(
				200,
				format!(r#"[{{"name":"proj_default","labels":{labels}}}]"#),
			)
		} else {
			(404, r#"{"message":"not used"}"#.to_string())
		}
	});
	let e = Engine::with_base_dir(fake.client(), "proj".to_string(), dir.to_path_buf());
	let logs = Logs::start();
	e.warn_if_project_used_elsewhere().await;
	logs.text()
}

#[tokio::test]
async fn a_project_created_from_another_directory_is_reported() {
	let here = tempfile::tempdir().unwrap();
	let text = warning_for(
		r#"{"podup.project":"proj","podup.working_dir":"/somewhere/else"}"#,
		here.path(),
	)
	.await;
	assert!(
		text.contains("was created from \"/somewhere/else\""),
		"{text}"
	);
	assert!(text.contains("down -v"), "{text}");
}

#[tokio::test]
async fn the_same_directory_or_no_record_says_nothing() {
	let here = tempfile::tempdir().unwrap();
	let canonical = std::fs::canonicalize(here.path()).unwrap();
	let same: &'static str = Box::leak(
		format!(
			r#"{{"podup.project":"proj","podup.working_dir":{}}}"#,
			serde_json::to_string(&canonical.to_string_lossy()).unwrap()
		)
		.into_boxed_str(),
	);
	assert!(!warning_for(same, here.path())
		.await
		.contains("was created from"));
	// Resources made before the label existed carry no directory.
	let old = warning_for(r#"{"podup.project":"proj"}"#, here.path()).await;
	assert!(!old.contains("was created from"), "{old}");
}

/// A symlink to the project directory is the same directory.
#[tokio::test]
async fn the_recorded_directory_is_canonical() {
	let real = tempfile::tempdir().unwrap();
	let links = tempfile::tempdir().unwrap();
	let link = links.path().join("via-link");
	std::os::unix::fs::symlink(real.path(), &link).unwrap();
	let fake = fake_podman::start(|_, _| (404, "{}".to_string()));
	let a = Engine::with_base_dir(fake.client(), "proj".into(), real.path().to_path_buf());
	let b = Engine::with_base_dir(fake.client(), "proj".into(), link);
	assert_eq!(a.working_dir_label(), b.working_dir_label());
}

/// The networks podup creates carry the directory, so a later run can compare.
#[tokio::test]
async fn a_created_network_records_the_directory() {
	let here = tempfile::tempdir().unwrap();
	let fake = fake_podman::start(|method, target| {
		if method == "POST" && target.ends_with("/networks/create") {
			(200, r#"{"name":"proj_n1"}"#.to_string())
		} else {
			(404, r#"{"message":"no such network"}"#.to_string())
		}
	});
	let e = Engine::with_base_dir(fake.client(), "proj".into(), here.path().to_path_buf());
	let file = crate::parse_str("services:\n  s:\n    image: x\nnetworks:\n  n1: {}\n").unwrap();
	e.create_networks(&file).await.unwrap();
	let bodies = fake.bodies.lock().unwrap();
	let created = bodies
		.iter()
		.map(|b| String::from_utf8_lossy(b).into_owned())
		.find(|b| b.contains("proj_n1"))
		.expect("a network create body");
	assert!(created.contains("podup.working_dir"), "{created}");
}

/// An engine whose project's one network carries `labels`.
fn engine_with_network(
	labels: &'static str,
	dir: &std::path::Path,
) -> (Engine, fake_podman::FakePodman) {
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/networks/json") {
			(
				200,
				format!(r#"[{{"name":"proj_default","labels":{labels}}}]"#),
			)
		} else {
			(404, r#"{"message":"not used"}"#.to_string())
		}
	});
	let e = Engine::with_base_dir(fake.client(), "proj".to_string(), dir.to_path_buf());
	(e, fake)
}

/// `down -v` from another directory would remove that project's volumes, so it
/// is refused unless `--force` says so.
#[tokio::test]
async fn a_destructive_down_from_another_directory_needs_force() {
	let here = tempfile::tempdir().unwrap();
	let (e, _fake) = engine_with_network(
		r#"{"podup.project":"proj","podup.working_dir":"/somewhere/else"}"#,
		here.path(),
	);
	let err = e
		.refuse_if_project_used_elsewhere("down -v", false)
		.await
		.expect_err("refused without --force");
	let text = err.to_string();
	assert!(
		text.contains("/somewhere/else") && text.contains("--force"),
		"{text}"
	);
	e.refuse_if_project_used_elsewhere("down -v", true)
		.await
		.expect("--force lets it run");
}

/// A project without the record (created before it existed) is not refused.
#[tokio::test]
async fn a_project_without_the_record_is_not_refused() {
	let here = tempfile::tempdir().unwrap();
	let (e, _fake) = engine_with_network(r#"{"podup.project":"proj"}"#, here.path());
	e.refuse_if_project_used_elsewhere("down -v", false)
		.await
		.expect("no record, no refusal");
}

/// After a `down` without `-v` the networks are gone and the volumes stay; the
/// volumes still carry the directory, so a later `down -v` from elsewhere is
/// refused.
#[tokio::test]
async fn a_volume_alone_still_records_the_directory() {
	let here = tempfile::tempdir().unwrap();
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/volumes/json") {
			(
				200,
				r#"[{"Name":"proj_data","Labels":{"podup.project":"proj","podup.working_dir":"/srv/a"}},
				   {"Name":"other_data","Labels":{"podup.project":"other","podup.working_dir":"/srv/x"}}]"#
					.to_string(),
			)
		} else if method == "GET" && target.contains("/json") {
			(200, "[]".to_string())
		} else {
			(404, r#"{"message":"not used"}"#.to_string())
		}
	});
	let e = Engine::with_base_dir(fake.client(), "proj".into(), here.path().to_path_buf());
	let err = e
		.refuse_if_project_used_elsewhere("down -v", false)
		.await
		.expect_err("the volume's record refuses it");
	assert!(err.to_string().contains("/srv/a"), "{err}");
}

/// The check decides whether `--force` is needed, so when the daemon cannot
/// answer it the teardown is refused instead of let through.
#[tokio::test]
async fn a_failed_listing_refuses_the_teardown() {
	let here = tempfile::tempdir().unwrap();
	let fake = fake_podman::start(|_, _| (500, r#"{"message":"boom"}"#.to_string()));
	let e = Engine::with_base_dir(fake.client(), "proj".into(), here.path().to_path_buf());
	let err = e
		.refuse_if_project_used_elsewhere("down -v", false)
		.await
		.expect_err("refused when the record cannot be read");
	assert!(err.to_string().contains("--force"), "{err}");
	e.refuse_if_project_used_elsewhere("down -v", true)
		.await
		.expect("--force still lets it run");
}

/// Two directories whose names differ only in bytes that are not UTF-8 keep
/// different labels.
#[tokio::test]
async fn directories_that_are_not_utf8_keep_distinct_labels() {
	use std::os::unix::ffi::OsStringExt;
	let root = tempfile::tempdir().unwrap();
	let a = root
		.path()
		.join(std::ffi::OsString::from_vec(b"dir-\xff".to_vec()));
	let b = root
		.path()
		.join(std::ffi::OsString::from_vec(b"dir-\xfe".to_vec()));
	if std::fs::create_dir(&a).is_err() || std::fs::create_dir(&b).is_err() {
		// A file system that refuses such names has nothing to collide.
		return;
	}
	let fake = fake_podman::start(|_, _| (404, "{}".to_string()));
	let la = Engine::with_base_dir(fake.client(), "proj".into(), a).working_dir_label();
	let lb = Engine::with_base_dir(fake.client(), "proj".into(), b).working_dir_label();
	assert_ne!(la, lb);
	assert!(la.ends_with("\\xff") && lb.ends_with("\\xfe"), "{la} {lb}");
}
