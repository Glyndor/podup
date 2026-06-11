//! Golden tests for compose -> Quadlet translation. They pin the exact unit
//! contents so any change to the mapping is reviewed deliberately.

use podup::parse_str;
use podup::quadlet::generate;

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
		"[Volume]\nVolumeName=myproj_cache\n\n[Install]\nWantedBy=default.target\n"
	);
	assert_eq!(
		unit(&out, "backend.network"),
		"[Network]\nNetworkName=myproj_backend\n\n[Install]\nWantedBy=default.target\n"
	);
}
