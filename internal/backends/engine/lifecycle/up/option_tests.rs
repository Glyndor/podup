use crate::engine::fake_podman::{self, FakeReply};
use crate::engine::Engine;

fn engine() -> Engine {
	let fake = fake_podman::start_replying(|_, _| FakeReply::Body(404, "{}".into()));
	Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir())
}

fn names(items: &[&str]) -> Vec<String> {
	items.iter().map(|s| s.to_string()).collect()
}

#[tokio::test]
async fn quiet_build_silences_the_builds_up_runs() {
	assert!(!engine().up_build_options().quiet);
	assert!(engine().with_quiet_build(true).up_build_options().quiet);
}

#[tokio::test]
async fn always_recreate_deps_spares_the_services_that_were_named() {
	let engine = engine().with_always_recreate_deps(true);
	let named = names(&["web"]);
	assert!(!engine.recreate_forced("web", &named, false));
	assert!(engine.recreate_forced("db", &named, false));
}

#[tokio::test]
async fn always_recreate_deps_has_nothing_to_single_out_when_all_are_selected() {
	let engine = engine().with_always_recreate_deps(true);
	assert!(!engine.recreate_forced("db", &[], false));
}

#[tokio::test]
async fn without_the_flag_only_force_recreate_forces() {
	let engine = engine();
	let named = names(&["web"]);
	assert!(!engine.recreate_forced("db", &named, false));
	assert!(engine.recreate_forced("db", &named, true));
}
