use crate::parse_str;
use crate::quadlet::generate_at;
use crate::quadlet::tests::unit_named;

#[test]
fn build_field_emits_a_build_unit() {
	let yaml = r#"
services:
  app:
    build:
      context: ./src
      dockerfile: Dockerfile.app
      target: runtime
    image: app:latest
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));
	// A `.build` unit is generated (no longer just a warning) and the container
	// references it so Quadlet builds before running.
	let build = out
		.units
		.iter()
		.find(|u| u.filename == "proj-app.build")
		.expect("a build service must emit an app.build unit");
	assert!(build.contents.contains("[Build]"));
	assert!(build.contents.contains("ImageTag=app:latest"));
	// `abs_context` joins with the OS separator, so build the expected path the
	// same way (`/srv/app/src` on Unix, `/srv/app\src` on Windows) rather than a
	// POSIX literal that fails the render tests (which are not Unix-gated) on Windows.
	let expected_ctx = format!(
		"SetWorkingDirectory={}",
		std::path::Path::new("/srv/app").join("src").display()
	);
	assert!(build.contents.contains(&expected_ctx), "{}", build.contents);
	assert!(build.contents.contains("File=Dockerfile.app"));
	assert!(build.contents.contains("Target=runtime"));
	let container = out
		.units
		.iter()
		.find(|u| u.filename == "proj-app.container")
		.unwrap();
	assert!(container.contents.contains("Image=proj-app.build"));
	// The 5.2-floor warning fires whenever a `.build` unit is written
	// (the floor for the unit type is 5.0); what must NOT fire here is
	// the inline-Dockerfile warning, which would mean we failed to emit
	// the unit.
	assert!(
		!out.warnings.iter().any(|w| w.contains("dockerfile_inline")),
		"inline-Dockerfile warning must not fire when the build is expressible; got: {:?}",
		out.warnings
	);
}

#[test]
fn inline_dockerfile_build_warns_and_emits_no_build_unit() {
	let yaml = "services:\n  app:\n    build:\n      dockerfile_inline: \"FROM alpine\"\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));
	assert!(!out.units.iter().any(|u| u.filename == "proj-app.build"));
	assert!(out.warnings.iter().any(|w| w.contains("dockerfile_inline")));
}

/// A service that sets none of the build fields the `.build` unit gained
/// renders the unit it rendered before: every new key is conditional, so a
/// build block naming only a context still produces the same three lines.
#[test]
fn a_build_without_the_new_fields_renders_the_same_unit_as_before() {
	let yaml = "services:\n  app:\n    build: .\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));
	let build = out
		.units
		.iter()
		.find(|u| u.filename == "proj-app.build")
		.expect("a build service emits a .build unit");
	assert_eq!(
		build.contents,
		concat!(
			"# podup-owner: proj\n",
			"[Build]\n",
			"ImageTag=proj-app\n",
			"SetWorkingDirectory=/srv/app\n",
			"Label=podup.project=proj\n",
		)
	);
}

/// #1970: `podup generate quadlet` (and `generate_at`) keep their standard
/// shape: a buildable service's `.container` references `<stem>.build` and
/// Quadlet adds the build dependency. The prebuilt shape that autostart needs
/// is a separate entry point (`generate_for_autostart`); the standard path
/// must not drift.
#[test]
fn generate_at_still_references_the_build_unit_for_a_buildable_service() {
	let yaml = "services:\n  web:\n    build: .\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));
	let container = unit_named(&out, "proj-web.container");
	assert!(
		container.contents.contains("Image=proj-web.build\n"),
		"standard path must point at the build unit; got:\n{}",
		container.contents
	);
	// The standard path must NOT have a `Pull=` either, since build: doesn't
	// imply any pull at all. Only the prebuilt path adds `Pull=never`.
	assert!(
		!container.contents.lines().any(|l| l.starts_with("Pull=")),
		"standard path must not emit a Pull= line; got:\n{}",
		container.contents
	);
}
