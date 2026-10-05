//! Unit tests for the `.pod` Quadlet unit emitted by `generate quadlet`
//! when the compose file declares `x-podman-pod: true`.

use crate::compose::parse_str;
use crate::quadlet::{generate_at, QuadletUnit};

fn unit_named<'a>(out: &'a crate::quadlet::QuadletOutput, filename: &str) -> &'a QuadletUnit {
	out.units
		.iter()
		.find(|u| u.filename == filename)
		.unwrap_or_else(|| panic!("no unit named {filename}"))
}

/// `generate quadlet` emits a single `<project>.pod` unit alongside the
/// per-service `.container` units when the extension is set, with the
/// project name as `PodName=`, one `Network=` per declared network, the
/// union of every service's `ports:` as `PublishPort=`, one
/// `--add-host=` flag per service, the project ownership label
/// (carried through `PodmanArgs=` because the native `AddHost=`/`Label=`
/// keys on `[Pod]` post-date the 5.0 floor), and `--exit-policy=continue`
/// (Quadlet hard-codes `--exit-policy stop` on the command line it builds;
/// the appended flag wins, and the pod keeps running when its last
/// service container exits, on every Quadlet-rendered unit). The
/// `.container` units reference it via `Pod=<stem>.pod` and drop their
/// own `PublishPort=` and `Network=` lines.
#[test]
fn quadlet_emits_a_pod_unit_and_moves_ports_to_it() {
	let yaml = r#"
x-podman-pod: true
services:
  web:
    image: nginx
    ports:
      - "127.0.0.1:8080:80"
    networks:
      - frontend
  db:
    image: postgres
    ports:
      - "5432:5432"
    environment:
      POSTGRES_PASSWORD: secret
networks:
  frontend:
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "demo", std::path::Path::new("/srv/app"));

	// One `.pod` unit named after the project.
	let pod = unit_named(&out, "demo.pod");
	assert!(
		pod.contents.contains("[Pod]"),
		"pod unit must carry a [Pod] section: {}",
		pod.contents
	);
	assert!(
		pod.contents.contains("PodName=demo"),
		"pod must be named after the project: {}",
		pod.contents
	);
	// Port union: 80/tcp on 127.0.0.1:8080 and 5432/tcp on 0.0.0.0:5432.
	assert!(
		pod.contents.contains("PublishPort=127.0.0.1:8080:80"),
		"pod must carry the web publish port: {}",
		pod.contents
	);
	assert!(
		pod.contents.contains("PublishPort=5432:5432"),
		"pod must carry the db publish port: {}",
		pod.contents
	);
	// A declared network is referenced through its generated `.network` unit,
	// the same way a `.container` unit references it outside pod mode.
	assert!(
		pod.contents.contains("Network=demo-frontend.network"),
		"pod must carry the declared network: {}",
		pod.contents
	);
	// One host entry per service, routed through `PodmanArgs=` because
	// `AddHost=` on `[Pod]` is a Podman 5.3.0 addition and the supported
	// floor is 5.0.
	assert!(
		pod.contents
			.contains("PodmanArgs=--add-host=\"web:127.0.0.1\""),
		"pod must carry the web host entry: {}",
		pod.contents
	);
	assert!(
		pod.contents
			.contains("PodmanArgs=--add-host=\"db:127.0.0.1\""),
		"pod must carry the db host entry: {}",
		pod.contents
	);
	// Ownership label, also routed through `PodmanArgs=` because
	// `Label=` on `[Pod]` is a Podman 5.6.0 addition.
	assert!(
		pod.contents
			.contains("PodmanArgs=--label=\"podup.project=demo\""),
		"pod must carry the ownership label: {}",
		pod.contents
	);
	// Exit policy: pinned to `continue` so the Quadlet-rendered pod agrees
	// with what the live engine stamps on the pod it creates. Quadlet
	// hard-codes `--exit-policy stop` in its `podman pod create` command
	// line; the appended flag wins, and the pod keeps running when its
	// last service container exits. Exactly one occurrence: a second
	// `--exit-policy=` would shadow the first and risk flip-flopping the
	// value on a Quadlet that processes the trailing flag differently.
	assert_eq!(
		pod.contents.matches("--exit-policy=continue").count(),
		1,
		"pod must carry exactly one `PodmanArgs=--exit-policy=continue`: {}",
		pod.contents
	);
	assert!(
		pod.contents.contains("PodmanArgs=--exit-policy=continue"),
		"pod must carry the exit-policy flag: {}",
		pod.contents
	);

	// Each `.container` unit references the pod and drops its own ports
	// and networks (the pod owns both).
	let web = unit_named(&out, "demo-web.container");
	assert!(
		web.contents.contains("Pod=demo.pod"),
		"container must reference the pod by name: {}",
		web.contents
	);
	assert!(
		!web.contents.contains("PublishPort="),
		"container inside a pod must not carry its own PublishPort=: {}",
		web.contents
	);
	assert!(
		!web.contents.contains("Network="),
		"container inside a pod must not carry its own Network=: {}",
		web.contents
	);

	let db = unit_named(&out, "demo-db.container");
	assert!(
		db.contents.contains("Pod=demo.pod"),
		"container must reference the pod by name: {}",
		db.contents
	);
	assert!(
		!db.contents.contains("PublishPort="),
		"container inside a pod must not carry its own PublishPort=: {}",
		db.contents
	);
}

/// `generate quadlet` for a project without the extension emits no `.pod`
/// unit and each `.container` unit keeps its own `PublishPort=` and
/// `Network=` lines (the pre-pod behaviour).
#[test]
fn quadlet_does_not_emit_a_pod_unit_without_the_extension() {
	let yaml = r#"
services:
  web:
    image: nginx
    ports:
      - "8080:80"
    networks:
      - frontend
networks:
  frontend:
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "demo", std::path::Path::new("/srv/app"));

	assert!(
		out.units.iter().all(|u| !u.filename.ends_with(".pod")),
		"no .pod unit must be emitted without the extension: {:?}",
		out.units.iter().map(|u| &u.filename).collect::<Vec<_>>()
	);

	let web = unit_named(&out, "demo-web.container");
	assert!(
		web.contents.contains("PublishPort=8080:80"),
		"container without pod mode keeps its own PublishPort=: {}",
		web.contents
	);
	assert!(
		web.contents.contains("Network=demo-frontend.network"),
		"container without pod mode keeps its own Network=: {}",
		web.contents
	);
}

/// When every service agrees on `network_mode: "pasta:-m,1400"` the pod
/// unit writes `Network=pasta:-m,1400` as-is, no `.network` reference
/// lands on it, and the per-service `.container` units drop `Network=`
/// entirely (they join the pod's namespace, the way the live engine
/// does). The mode-with-options is preserved on the pod so Podman's CLI
/// sees the same string it would for `podman pod create --network
/// pasta:-m,1400` and splits it into `netns` and `network_options`.
#[test]
fn quadlet_pod_unit_carries_agreed_pasta_mode_with_options() {
	let yaml = r#"
x-podman-pod: true
services:
  web:
    image: nginx
    network_mode: "pasta:-m,1400"
  db:
    image: postgres
    network_mode: "pasta:-m,1400"
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "demo", std::path::Path::new("/srv/app"));

	let pod = unit_named(&out, "demo.pod");
	assert!(
		pod.contents.contains("Network=pasta:-m,1400"),
		"pod must carry the agreed pasta mode verbatim: {}",
		pod.contents
	);
	assert!(
		!pod.contents.contains(".network"),
		"pod on pasta must not reference a generated network unit: {}",
		pod.contents
	);
	let web = unit_named(&out, "demo-web.container");
	assert!(
		web.contents.contains("Pod=demo.pod"),
		"container must reference the pod: {}",
		web.contents
	);
	assert!(
		!web.contents.contains("Network="),
		"container inside a pod on pasta must not carry its own Network=: {}",
		web.contents
	);
}
