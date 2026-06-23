//! Golden tests for compose -> Quadlet translation. They pin the exact unit
//! contents so any change to the mapping is reviewed deliberately.

use std::fs;
use std::process::Command;

use podup::parse_str;
use podup::quadlet::generate;

fn bin() -> &'static str {
	env!("CARGO_BIN_EXE_podup")
}

fn unit<'a>(out: &'a podup::quadlet::QuadletOutput, name: &str) -> &'a str {
	&out.units
		.iter()
		.find(|u| u.filename == name)
		.unwrap_or_else(|| panic!("missing unit {name}"))
		.contents
}

#[test]
fn container_unit_matches_golden() {
	let yaml = r#"
services:
  web:
    image: nginx:1.27
    container_name: web
    ports:
      - "8080:80"
    environment:
      TZ: UTC
    volumes:
      - data:/var/lib/data
    networks:
      - frontend
    restart: always
    depends_on:
      - db
volumes:
  data:
networks:
  frontend:
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate(&file, "proj");

	let expected = "\
[Unit]
Description=web (podup)
After=db.service
Requires=db.service

[Container]
ContainerName=web
Image=nginx:1.27
PublishPort=8080:80
Environment=TZ=UTC
Volume=data.volume:/var/lib/data
Network=frontend.network
Label=podup.project=proj
Label=podup.service=web

[Service]
Restart=always

[Install]
WantedBy=default.target
";
	assert_eq!(unit(&out, "web.container"), expected);
}

#[test]
fn network_and_volume_units_match_golden() {
	let yaml = r#"
services:
  app:
    image: alpine
volumes:
  cache:
networks:
  backend:
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate(&file, "myproj");

	assert_eq!(
		unit(&out, "cache.volume"),
		"[Volume]\nVolumeName=myproj_cache\nLabel=podup.project=myproj\n"
	);
	assert_eq!(
		unit(&out, "backend.network"),
		"[Network]\nNetworkName=myproj_backend\nLabel=podup.project=myproj\n"
	);
}

#[test]
fn cli_writes_units_to_output_dir() {
	let dir = std::env::temp_dir().join(format!("podup-quadlet-{}", std::process::id()));
	let _ = fs::remove_dir_all(&dir);
	let compose = dir.join("docker-compose.yml");
	fs::create_dir_all(&dir).unwrap();
	fs::write(
		&compose,
		"services:\n  web:\n    image: nginx\nvolumes:\n  data:\n",
	)
	.unwrap();
	let out_dir = dir.join("units");

	let status = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "generate", "quadlet", "-o"])
		.arg(&out_dir)
		.status()
		.unwrap();
	assert!(status.success());

	let web = fs::read_to_string(out_dir.join("web.container")).unwrap();
	assert!(web.contains("Image=nginx"));
	assert!(fs::read_to_string(out_dir.join("data.volume"))
		.unwrap()
		.contains("VolumeName="));

	let _ = fs::remove_dir_all(&dir);
}

#[test]
fn cli_prints_units_to_stdout() {
	let dir = std::env::temp_dir().join(format!("podup-quadlet-stdout-{}", std::process::id()));
	let _ = fs::remove_dir_all(&dir);
	fs::create_dir_all(&dir).unwrap();
	let compose = dir.join("docker-compose.yml");
	fs::write(&compose, "services:\n  web:\n    image: nginx\n").unwrap();

	let output = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "generate", "quadlet"])
		.output()
		.unwrap();
	assert!(output.status.success());
	let stdout = String::from_utf8(output.stdout).unwrap();
	assert!(stdout.contains("# web.container"));
	assert!(stdout.contains("Image=nginx"));

	let _ = fs::remove_dir_all(&dir);
}
