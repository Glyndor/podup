//! The project budget holds when the library is entered by a single file.
//!
//! The command line always loads through the list of files, whose own scope
//! would hide a missing one in the single-file entry points. These call
//! `parse_file_with_env_files` and `parse_file_with_env_files_interp`
//! directly, with `include:` files of 1 MiB each: 15 MiB loads, 17 MiB does not.

use std::fs;
use std::path::PathBuf;

use podup::compose::{parse_file_with_env_files, parse_file_with_env_files_interp};

const MIB: usize = 1024 * 1024;

/// A project whose root includes `count` files, each repeating `${BIG}` once.
fn project(count: usize) -> (tempfile::TempDir, PathBuf) {
	let dir = tempfile::tempdir().expect("create temp dir");
	fs::write(
		dir.path().join(".env"),
		format!("BIG={}\n", "x".repeat(MIB)),
	)
	.unwrap();
	let mut names = Vec::new();
	for index in 0..count {
		let name = format!("f{index}.yaml");
		let body =
			format!("services:\n  s{index}:\n    image: alpine\n    command: [\"${{BIG}}\"]\n");
		fs::write(dir.path().join(&name), body).unwrap();
		names.push(name);
	}
	let root = dir.path().join("compose.yaml");
	fs::write(&root, format!("include: [{}]\n", names.join(", "))).unwrap();
	(dir, root)
}

fn assert_budget_refusal(error: impl std::fmt::Display) {
	let text = error.to_string();
	assert!(text.contains("bytes of substituted output"), "{text}");
}

#[test]
fn parse_file_with_env_files_shares_the_budget_across_includes() {
	let (_dir, root) = project(15);
	parse_file_with_env_files(&root, &[]).expect("15 MiB is under the budget");
	let (_dir, root) = project(17);
	assert_budget_refusal(parse_file_with_env_files(&root, &[]).expect_err("17 MiB is over"));
}

#[test]
fn parse_file_with_env_files_interp_shares_the_budget_across_includes() {
	let (_dir, root) = project(15);
	parse_file_with_env_files_interp(&root, &[], true).expect("15 MiB is under the budget");
	let (_dir, root) = project(17);
	assert_budget_refusal(
		parse_file_with_env_files_interp(&root, &[], true).expect_err("17 MiB is over"),
	);
}

/// A subscriber that, on its first event, runs `action`, which loads something
/// through the public API. That is a synchronous callback in the middle of a load.
mod reentrant {
	use std::sync::atomic::{AtomicBool, Ordering};
	use std::sync::{Arc, Mutex};

	use tracing::span::{Attributes, Id, Record};
	use tracing::{Event, Metadata, Subscriber};

	pub struct Loads {
		pub action: Box<dyn Fn() -> Result<(), String> + Send + Sync>,
		pub fired: AtomicBool,
		pub result: Arc<Mutex<Option<Result<(), String>>>>,
	}

	impl Subscriber for Loads {
		fn enabled(&self, _: &Metadata<'_>) -> bool {
			true
		}
		fn new_span(&self, _: &Attributes<'_>) -> Id {
			Id::from_u64(1)
		}
		fn record(&self, _: &Id, _: &Record<'_>) {}
		fn record_follows_from(&self, _: &Id, _: &Id) {}
		fn enter(&self, _: &Id) {}
		fn exit(&self, _: &Id) {}
		fn event(&self, _: &Event<'_>) {
			if !self.fired.swap(true, Ordering::SeqCst) {
				*self.result.lock().unwrap() = Some((self.action)());
			}
		}
	}
}

/// 15 MiB in `a.yaml`, then a variable nobody defined in `b.yaml`: the warning
/// for it is emitted while the 15 MiB are still counted.
fn project_that_warns_late() -> (tempfile::TempDir, PathBuf) {
	let (dir, root) = project(0);
	let item = "\"${BIG}\", ";
	let body = format!(
		"services:\n  a:\n    image: alpine\n    command: [{}]\n",
		item.repeat(15)
	);
	fs::write(dir.path().join("a.yaml"), body).unwrap();
	let late = "services:\n  b:\n    image: alpine\n    command: [\"${UNDEFINED_ON_PURPOSE}\"]\n";
	fs::write(dir.path().join("b.yaml"), late).unwrap();
	fs::write(&root, "include: [a.yaml, b.yaml]\n").unwrap();
	(dir, root)
}

/// Load the project in `outer` with `action` running from its late warning, and
/// return what `action` got.
fn loaded_from_a_callback(
	outer: &std::path::Path,
	action: impl Fn() -> Result<(), String> + Send + Sync + 'static,
) -> Result<(), String> {
	let result = std::sync::Arc::new(std::sync::Mutex::new(None));
	let subscriber = reentrant::Loads {
		action: Box::new(action),
		fired: std::sync::atomic::AtomicBool::new(false),
		result: result.clone(),
	};
	let loaded =
		tracing::subscriber::with_default(subscriber, || parse_file_with_env_files(outer, &[]));
	loaded.expect("15 MiB fits alone");
	let inner_result = result.lock().unwrap().take().expect("the callback ran");
	inner_result
}

/// Two projects that each fit, the second loaded from a callback of the first.
/// A load that starts through the public API has a budget of its own.
#[test]
fn a_load_started_from_a_callback_has_its_own_budget() {
	let (_inner_dir, inner) = project(2);
	parse_file_with_env_files(&inner, &[]).expect("2 MiB fits alone");
	let (_outer_dir, outer) = project_that_warns_late();
	let got = loaded_from_a_callback(&outer, move || {
		parse_file_with_env_files(&inner, &[])
			.map(|_| ())
			.map_err(|e| e.to_string())
	});
	assert_eq!(
		got,
		Ok(()),
		"the second project borrowed the first one's budget"
	);
}

/// The same for a document handed over as text, which has no files to read.
#[test]
fn a_document_parsed_from_a_callback_has_its_own_budget() {
	// Two defaults of 1 MiB each, written in the document itself because the
	// text entry point reads the variables of the process, not of a project.
	let big = "y".repeat(MIB);
	let document = format!(
		"services:\n  s:\n    image: alpine\n    command: [\"${{NOPE:-{big}}}\", \"${{NOPE:-{big}}}\"]\n"
	);
	podup::compose::parse_str(&document).expect("2 MiB fits alone");
	let (_outer_dir, outer) = project_that_warns_late();
	let got = loaded_from_a_callback(&outer, move || {
		podup::compose::parse_str(&document)
			.map(|_| ())
			.map_err(|e| e.to_string())
	});
	assert_eq!(got, Ok(()), "the document borrowed the project's budget");
}
