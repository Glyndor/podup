use crate::parse_str;
use crate::quadlet::tests::unit_named;
use crate::quadlet::{generate_at, validate_for_quadlet};

#[test]
fn service_network_uses_the_exported_container_name() {
	for (identity, expected) in [("", "p-db"), ("container_name: database", "database")] {
		let yaml = format!(
			"services:\n  db:\n    image: postgres\n    {identity}\n  web:\n    image: nginx\n    network_mode: service:db\n"
		);
		let file = parse_str(&yaml).unwrap();
		validate_for_quadlet(&file).unwrap();
		let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
		let db = &unit_named(&out, "p-db.container").contents;
		let web = &unit_named(&out, "p-web.container").contents;
		assert!(db.contains(&format!("ContainerName={expected}\n")), "{db}");
		assert!(
			web.contains(&format!("Network=container:{expected}\n")),
			"{web}"
		);
		let unit = web.split("[Container]").next().unwrap();
		assert_eq!(unit.matches("After=p-db.service\n").count(), 1, "{web}");
		assert_eq!(unit.matches("Requires=p-db.service\n").count(), 1, "{web}");
	}
}

#[test]
fn service_network_requires_even_an_optional_explicit_dependency_once() {
	for depends_on in [
		"depends_on: [db]",
		"depends_on:\n      db:\n        condition: service_started\n        required: false",
	] {
		let yaml = format!(
			"services:\n  db:\n    image: postgres\n  web:\n    image: nginx\n    network_mode: service:db\n    {depends_on}\n"
		);
		let file = parse_str(&yaml).unwrap();
		let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
		let web = &unit_named(&out, "p-web.container").contents;
		let unit = web.split("[Container]").next().unwrap();
		assert_eq!(unit.matches("After=p-db.service\n").count(), 1, "{web}");
		assert_eq!(unit.matches("Requires=p-db.service\n").count(), 1, "{web}");
		assert!(!unit.contains("Wants=p-db.service\n"), "{web}");
	}
}

#[test]
fn service_network_rejects_an_undefined_target_before_export() {
	let file =
		parse_str("services:\n  web:\n    image: nginx\n    network_mode: service:missing\n")
			.unwrap();
	let error = validate_for_quadlet(&file).unwrap_err().to_string();
	assert!(error.contains("web"), "{error}");
	assert!(error.contains("network_mode"), "{error}");
	assert!(error.contains("missing"), "{error}");
}
