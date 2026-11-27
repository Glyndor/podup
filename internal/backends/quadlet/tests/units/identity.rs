use crate::parse_str;
use crate::quadlet::tests::unit_named;
use crate::quadlet::{generate_at, QuadletOutput, QuadletUnit};

#[test]
fn container_name_defaults_to_project_prefixed() {
	// A service with no explicit `container_name:` must default to
	// `{project}-{service}`, matching how `up` names the running container,
	// rather than a bare `web` that would collide across projects.
	let file = parse_str("services:\n  web:\n    image: nginx\n").unwrap();
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));
	let web = unit_named(&out, "proj-web.container");
	assert!(web.contents.contains("ContainerName=proj-web"));
}

#[test]
fn duplicate_filename_detects_collision() {
	let mk = |n: &str| QuadletUnit {
		filename: n.to_string(),
		contents: String::new(),
	};
	let mut out = QuadletOutput {
		units: vec![mk("web.container"), mk("db.volume")],
		..Default::default()
	};
	assert_eq!(out.duplicate_filename(), None);
	out.units.push(mk("web.container"));
	assert_eq!(out.duplicate_filename(), Some("web.container"));
}

#[test]
fn two_files_that_make_one_service_are_a_collision() {
	let mk = |n: &str| QuadletUnit {
		filename: n.to_string(),
		contents: String::new(),
	};
	for (first, second) in [
		("app-web.build", "app-web-build.container"),
		("app-web.network", "app-web-network.container"),
		("app-web.volume", "app-web-volume.container"),
		("app-web.pod", "app-web-pod.container"),
		("app-web.image", "app-web-image.container"),
		("app-web.container", "app-web.service"),
		("app-web.kube", "app-web.container"),
		("app-web.build", "app-web-build.service"),
	] {
		for units in [[first, second], [second, first]] {
			let out = QuadletOutput {
				units: units.into_iter().map(mk).collect(),
				..Default::default()
			};
			assert_eq!(out.duplicate_filename(), Some(units[1]), "{units:?}");
		}
	}
}

#[test]
fn files_that_make_different_services_are_no_collision() {
	let mk = |n: &str| QuadletUnit {
		filename: n.to_string(),
		contents: String::new(),
	};
	// A timer names the service it starts, and is no service of its own.
	let out = QuadletOutput {
		units: [
			"app-web.build",
			"app-web.container",
			"app-web.network",
			"app-web.volume",
			"app-web.pod",
			"app-job-1.timer",
			"app-job-1.container",
		]
		.into_iter()
		.map(mk)
		.collect(),
		..Default::default()
	};
	assert_eq!(out.duplicate_filename(), None);
}

#[test]
fn a_service_named_like_a_build_collides_with_it() {
	let yaml = "services:\n  web:\n    build: .\n  web-build:\n    image: x\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "app", std::path::Path::new("/srv/app"));
	assert_eq!(out.duplicate_filename(), Some("app-web-build.container"));
	let yaml = "services:\n  web:\n    build: .\n  api:\n    image: x\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "app", std::path::Path::new("/srv/app"));
	assert_eq!(out.duplicate_filename(), None);
}

#[test]
fn hostile_service_name_cannot_escape_output_directory() {
	// A compose key containing path separators must never yield a unit
	// file name that escapes the output directory.
	let yaml = "services:\n  ? \"../../evil\"\n  : { image: x }\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));
	let unit = &out.units[0];
	assert!(
		!unit.filename.contains('/') && !unit.filename.contains('\\'),
		"unit file name must be a single safe component, got {}",
		unit.filename
	);
	assert!(unit.filename.ends_with(".container"));
}

#[test]
fn units_carry_podup_ownership_labels() {
	// Every generated unit must carry the same ownership labels the live engine
	// stamps: `podup.project` on all three unit types and `podup.service` on the
	// container, so exported resources are traceable back to their project.
	let yaml = r#"
services:
  web:
    image: nginx:1.27
networks:
  net:
volumes:
  vol:
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));

	let c = &unit_named(&out, "proj-web.container").contents;
	assert!(
		c.contains("Label=podup.project=proj"),
		"container missing project ownership label in:\n{c}"
	);
	assert!(
		c.contains("Label=podup.service=web"),
		"container missing service ownership label in:\n{c}"
	);

	let net = &unit_named(&out, "proj-net.network").contents;
	assert!(
		net.contains("Label=podup.project=proj"),
		"network missing project ownership label in:\n{net}"
	);
	// Networks/volumes are project-scoped, not service-scoped: no service label.
	assert!(
		!net.contains("podup.service"),
		"network must not carry a service label in:\n{net}"
	);

	let vol = &unit_named(&out, "proj-vol.volume").contents;
	assert!(
		vol.contains("Label=podup.project=proj"),
		"volume missing project ownership label in:\n{vol}"
	);
	assert!(
		!vol.contains("podup.service"),
		"volume must not carry a service label in:\n{vol}"
	);
}
