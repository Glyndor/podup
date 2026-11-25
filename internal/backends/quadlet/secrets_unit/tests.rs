use std::path::Path;

use crate::compose::parse_str;
use crate::quadlet::{generate_at, QuadletOutput};

pub(super) const BASE: &str = "/srv/app";

pub(super) fn generate(yaml: &str) -> QuadletOutput {
	generate_at(&parse_str(yaml).unwrap(), "proj", Path::new(BASE))
}

/// Whether the output creates no secret: there is no secrets unit, or it holds
/// nothing but commands that fail.
pub(super) fn creates_nothing(out: &QuadletOutput) -> bool {
	secrets_unit(out).is_none_or(|unit| !unit.contains("secret create"))
}

pub(super) fn secrets_unit(out: &QuadletOutput) -> Option<&str> {
	out.units
		.iter()
		.find(|u| u.filename == "proj-podup-secrets.service")
		.map(|u| u.contents.as_str())
}

/// A path as a compose scalar: quoted, and with `$` doubled so that interpolation
/// leaves it as written.
pub(super) fn yaml_path(path: &str) -> String {
	serde_json::to_string(&path.replace('$', "$$")).unwrap()
}

fn container<'a>(out: &'a QuadletOutput, name: &str) -> &'a str {
	&out.units
		.iter()
		.find(|u| u.filename == format!("proj-{name}.container"))
		.unwrap()
		.contents
}

pub(super) const FILE_SECRET: &str = "services:\n  web:\n    image: img\n    secrets: [token]\nsecrets:\n  token:\n    file: ./token.txt\n";

// Reads a file of the host and its mode, which a Windows host does not have, and
// the path of a host file is not one of the machine the units run on.
#[cfg(unix)]
#[test]
fn a_file_secret_is_created_by_one_unit_that_holds_the_path_and_not_the_bytes() {
	let dir = tempfile::tempdir().unwrap();
	let file = dir.path().join("token.txt");
	std::fs::write(&file, "the-secret-bytes").unwrap();
	// Not the mode the umask of whoever runs the test gives it.
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
	}
	let yaml = FILE_SECRET.replace("./token.txt", file.to_str().unwrap());
	let out = generate(&yaml);
	let unit = secrets_unit(&out).expect("no secrets unit");
	assert!(unit.starts_with("# podup-owner: proj\n"));
	assert!(unit.contains("Type=oneshot\nRemainAfterExit=yes\n"));
	assert!(
		unit.contains("ExecStart=/bin/sh -c \"set -eu;")
			&& unit.contains(&format!(
				"\" podup-secret \"proj_secret_token\" \"proj\" \"{}\" \"0644\"\n",
				file.display()
			))
	);
	assert!(!unit.contains("the-secret-bytes"));
	assert!(!out.warnings.iter().any(|w| w.contains("token")));
}

#[test]
fn a_relative_file_is_resolved_against_the_project_directory() {
	let out = generate(FILE_SECRET);
	let unit = secrets_unit(&out).unwrap();
	assert!(unit.contains("\"/srv/app/token.txt\""));
}

#[test]
fn the_container_requires_the_unit_and_keeps_its_secret() {
	let out = generate(FILE_SECRET);
	let web = container(&out, "web");
	assert!(web.contains("Requires=proj-podup-secrets.service\n"));
	assert!(web.contains("After=proj-podup-secrets.service\n"));
	assert!(web.contains("Secret=proj_secret_token,mode=400\n"));
}

#[test]
fn a_service_that_mounts_nothing_exported_does_not_wait_for_the_unit() {
	let out = generate(
		"services:\n  web:\n    image: img\n    secrets: [token]\n  other:\n    image: img\nsecrets:\n  token:\n    file: ./token.txt\n",
	);
	assert!(!container(&out, "other").contains("podup-secrets"));
}

#[test]
fn two_services_sharing_a_secret_share_one_create() {
	let out = generate(
		"services:\n  a:\n    image: img\n    secrets: [token]\n  b:\n    image: img\n    secrets: [token]\nsecrets:\n  token:\n    file: ./t\n",
	);
	let unit = secrets_unit(&out).unwrap();
	assert_eq!(unit.matches("secret create").count(), 1);
	assert!(container(&out, "a").contains("Requires=proj-podup-secrets.service"));
	assert!(container(&out, "b").contains("Requires=proj-podup-secrets.service"));
}

#[test]
fn a_secret_nobody_mounts_creates_nothing() {
	let out = generate("services:\n  web:\n    image: img\nsecrets:\n  unused:\n    file: ./t\n");
	assert!(secrets_unit(&out).is_none());
}

#[test]
fn a_config_is_created_and_mounted_where_up_mounts_it() {
	let out = generate(
		"services:\n  web:\n    image: img\n    configs:\n      - app\n      - source: other\n        target: /etc/other.conf\n        uid: '1000'\n        mode: 0o440\nconfigs:\n  app:\n    file: ./app.conf\n  other:\n    file: ./other.conf\n",
	);
	let unit = secrets_unit(&out).unwrap();
	assert!(unit.contains("\"proj_config_app\" \"proj\" \"/srv/app/app.conf\""));
	assert!(unit.contains("\"proj_config_other\" \"proj\" \"/srv/app/other.conf\""));
	let web = container(&out, "web");
	assert!(web.contains("Secret=proj_config_app,type=mount,target=/app,mode=400\n"));
	assert!(web.contains(
		"Secret=proj_config_other,type=mount,target=/etc/other.conf,uid=1000,mode=440\n"
	));
	assert!(web.contains("Requires=proj-podup-secrets.service"));
	assert!(!out.warnings.iter().any(|w| w.contains("configs")));
}

/// A secret and a config with the same compose name are two Podman secrets.
#[test]
fn a_secret_and_a_config_of_one_name_do_not_collide() {
	let out = generate(
		"services:\n  web:\n    image: img\n    secrets: [x]\n    configs: [x]\nsecrets:\n  x:\n    file: ./s\nconfigs:\n  x:\n    file: ./c\n",
	);
	let unit = secrets_unit(&out).unwrap();
	assert!(unit.contains("\"proj_secret_x\"") && unit.contains("\"proj_config_x\""));
}

#[test]
fn an_external_secret_is_not_ours_to_create() {
	let out = generate(
		"services:\n  web:\n    image: img\n    secrets: [ext]\nsecrets:\n  ext:\n    external: true\n    name: pre_made\n",
	);
	assert!(secrets_unit(&out).is_none());
	let web = container(&out, "web");
	assert!(web.contains("Secret=pre_made\n"));
	assert!(!web.contains("podup-secrets"));
}

#[test]
fn an_environment_or_inline_secret_is_left_to_the_operator_and_the_unit_has_no_value() {
	let out = generate(
		"services:\n  web:\n    image: img\n    secrets: [e, c]\nsecrets:\n  e:\n    environment: DB_PASSWORD\n  c:\n    content: hunter2-literal\n",
	);
	assert!(secrets_unit(&out).is_none());
	let warnings = out.warnings.join("\n");
	assert!(warnings.contains(
		"printenv DB_PASSWORD | podman secret create --label podup.project=proj proj_secret_e -"
	));
	assert!(warnings.contains("podman secret create --label podup.project=proj proj_secret_c -"));
	assert!(!warnings.contains("hunter2-literal"));
	for unit in &out.units {
		assert!(!unit.contents.contains("hunter2-literal"));
	}
	// The container still names them, so it does not start without them.
	let web = container(&out, "web");
	assert!(web.contains("Secret=proj_secret_e\n") && web.contains("Secret=proj_secret_c\n"));
	assert!(!web.contains("podup-secrets"));
}

// --- hostile paths ---------------------------------------------------------

fn unit_for_path(path: &str) -> (Option<String>, Vec<String>) {
	let yaml = format!(
		"services:\n  web:\n    image: img\n    secrets: [token]\nsecrets:\n  token:\n    file: {}\n",
		yaml_path(path)
	);
	let out = generate(&yaml);
	(secrets_unit(&out).map(str::to_string), out.warnings)
}

#[test]
fn a_path_with_blanks_percent_dollar_quotes_and_backslashes_is_one_literal_word() {
	let (unit, _) = unit_for_path("/srv/my dir/50%/$HOME/a\"b'c\\d");
	let unit = unit.expect("the path is valid");
	assert!(unit.contains("\"/srv/my dir/50%%/$$HOME/a\\\"b'c\\\\d\" \"\"\n"));
	assert_eq!(unit.matches("ExecStart=").count(), 1);
}

#[test]
fn a_path_with_a_newline_or_any_control_character_is_refused() {
	for path in [
		"/srv/a\nExecStart=/bin/true",
		"/srv/a\rb",
		"/srv/a\u{7}b",
		"/srv/a\u{0}b",
	] {
		let (unit, warnings) = unit_for_path(path);
		assert!(
			unit.as_deref().is_none_or(|u| !u.contains("secret create")),
			"produced a unit"
		);
		assert!(warnings.iter().any(|w| w.contains("control character")));
	}
}

#[test]
fn a_hostile_name_cannot_reach_the_command_line() {
	let yaml = "services:\n  web:\n    image: img\n    secrets: ['a b; reboot']\nsecrets:\n  'a b; reboot':\n    file: ./t\n";
	let out = generate(yaml);
	assert!(creates_nothing(&out));
	assert!(out.warnings.iter().any(|w| w.contains("rename it")));
}

/// A service named like the unit would take its file name.
#[test]
fn a_service_with_the_units_name_keeps_its_own_unit() {
	let out = generate(
		"services:\n  podup-secrets:\n    image: img\n    secrets: [token]\nsecrets:\n  token:\n    file: ./t\n",
	);
	assert!(secrets_unit(&out).is_none());
	assert!(out
		.warnings
		.iter()
		.any(|w| w.contains("rename the service")));
	assert!(out
		.units
		.iter()
		.any(|u| u.filename == "proj-podup-secrets.container"));
}

// --- what `autostart --mode quadlet` installs does not change ------------------

/// A stack that starts under `autostart --mode quadlet` today must keep
/// starting. Its units are generated in the prebuilt shape, which has no secrets
/// unit yet (nothing there would install it), so the secrets unit, the
/// `Requires=` on it and the mounted configs stay out of it. The golden text is
/// what `podup autostart install --mode quadlet --dry-run` printed on `develop`
/// at 67ae1b9b for the same file (units, then warnings, in order), taken before
/// this change. A secret with a file source, an inline one, an external one and
/// two configs are all in it.
#[test]
fn the_units_autostart_installs_are_the_ones_it_installed_before() {
	// Parsed the way the command parses it, which adds the project's default
	// network that the units name.
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("compose.yaml");
	std::fs::write(&path, AUTOSTART_COMPOSE).unwrap();
	let file = crate::compose::parse_files_with_project_in(
		&[path],
		&[],
		true,
		Some(dir.path()),
		Some("proj"),
	)
	.unwrap();
	let out = crate::quadlet::generate_for_autostart(&file, "proj", Path::new("/srv/app"));
	let mut text = String::new();
	for unit in &out.units {
		text.push_str(&format!("# {}\n{}", unit.filename, unit.contents));
	}
	text.push_str("--- warnings\n");
	for warning in &out.warnings {
		text.push_str(warning);
		text.push('\n');
	}
	assert_eq!(text, AUTOSTART_BEFORE);
}

/// The same file, exported for `generate quadlet`, does get the unit: the golden
/// test above is about the prebuilt shape and only that.
#[test]
fn the_same_file_exported_for_generate_gets_the_secrets_unit() {
	let out = generate(AUTOSTART_COMPOSE);
	let unit = secrets_unit(&out).expect("no secrets unit");
	assert!(unit.contains("\"proj_secret_filesec\""));
	assert!(unit.contains("\"proj_config_appcfg\""));
	assert!(!unit.contains("proj_secret_inl") && !unit.contains("proj_config_other"));
	assert!(container(&out, "web").contains("Requires=proj-podup-secrets.service"));
}

// --- against the real tools ---------------------------------------------------

/// The compose file of the golden test.
const AUTOSTART_COMPOSE: &str = r#"services:
  web:
    image: nginx:1.27
    secrets:
      - filesec
      - source: inl
        target: inline.txt
        mode: 0o400
      - extsec
    configs:
      - appcfg
      - source: other
        target: /etc/other.conf
        uid: "1000"
        mode: 0o440
  worker:
    image: busybox:latest
    secrets: [filesec]
secrets:
  filesec:
    file: ./filesec.txt
  inl:
    content: not-in-units
  extsec:
    external: true
configs:
  appcfg:
    file: ./app.conf
  other:
    content: hello
"#;

/// What `autostart --mode quadlet` produced for it before this change.
const AUTOSTART_BEFORE: &str = r#"# proj-default.network
# podup-owner: proj
[Network]
NetworkName=proj_default
Options=isolate=true
Label=podup.project=proj
# proj-web.container
# podup-owner: proj
[Unit]
Description=web (podup)

[Container]
ContainerName=proj-web
Image=nginx:1.27
Network=proj-default.network:alias=web
Label=podup.project=proj
Label=podup.service=web
LogDriver=k8s-file
LogOpt=max-size=10485760
Secret=proj_secret_filesec
Secret=proj_secret_inl,target=inline.txt,mode=400
Secret=extsec

[Install]
WantedBy=default.target
# proj-worker.container
# podup-owner: proj
[Unit]
Description=worker (podup)

[Container]
ContainerName=proj-worker
Image=busybox:latest
Network=proj-default.network:alias=worker
Label=podup.project=proj
Label=podup.service=worker
LogDriver=k8s-file
LogOpt=max-size=10485760
Secret=proj_secret_filesec

[Install]
WantedBy=default.target
--- warnings
web: secret "filesec" is referenced but Quadlet does not create it; provision the project-scoped secret "proj_secret_filesec" first (e.g. via `podup up`)
web: secret "inl" is referenced but Quadlet does not create it; provision the project-scoped secret "proj_secret_inl" first (e.g. via `podup up`)
web: configs have no Quadlet equivalent and are skipped
worker: secret "filesec" is referenced but Quadlet does not create it; provision the project-scoped secret "proj_secret_filesec" first (e.g. via `podup up`)
"#;

// --- the script ---------------------------------------------------------------

fn exec_lines_of(yaml: &str) -> Vec<String> {
	let out = generate_at(&parse_str(yaml).unwrap(), "proj", Path::new("/srv/app"));
	out.units
		.iter()
		.find(|u| u.filename.ends_with("podup-secrets.service"))
		.unwrap()
		.contents
		.lines()
		.filter(|l| l.starts_with("ExecStart="))
		.map(str::to_string)
		.collect()
}

/// One constant for every secret: two secrets differ in their arguments only.
#[test]
fn the_script_is_one_constant_and_only_the_arguments_change() {
	let lines = exec_lines_of(
		"services:\n  web:\n    image: img\n    secrets: [a, b]\nsecrets:\n  a:\n    file: ./one\n  b:\n    file: ./two\n",
	);
	assert_eq!(lines.len(), 2);
	let script = super::exec_word(super::CREATE_SCRIPT).unwrap();
	let prefix = format!("ExecStart=/bin/sh -c {script} podup-secret ");
	for line in &lines {
		assert!(line.starts_with(&prefix));
	}
	assert_ne!(lines[0], lines[1]);
	assert!(
		lines[0].ends_with("\"/srv/app/one\" \"\"") && lines[1].ends_with("\"/srv/app/two\" \"\"")
	);
}

#[test]
fn the_script_stops_on_error_quotes_every_argument_and_ends_every_option_list() {
	let script = super::CREATE_SCRIPT;
	assert!(script.starts_with("set -eu;"));
	assert!(
		super::REFUSE_SCRIPT.starts_with("set -eu;") && super::REFUSE_SCRIPT.ends_with("exit 1")
	);
	assert!(script.contains("exists -- \"$1\""));
	assert!(script.contains("--format '{{index .Spec.Labels \"podup.project\"}}' -- \"$1\""));
	assert!(script.contains("--label \"podup.project=$2\" -- \"$1\" - <&3"));
	assert!(script.contains("exec 3< \"$3\"; [ -f /dev/fd/3 ]"));
	// A parameter is read unquoted nowhere, except the mode inside the arithmetic,
	// which the export writes as octal digits and nothing else; the messages are
	// the places a parameter sits inside a longer quote.
	let messages = [
		"echo \"podup: $3 is not a regular file (a link, a directory, a pipe or a device is refused); not creating $1\"",
		"echo \"podup: $3 is not the regular file it was when it was opened; not creating $1\"",
		"echo \"podup: $3 has mode $m, and had $4 when the units were generated; not creating $1; generate the units again\"",
		"echo \"podup: a secret named $1 exists and is not labelled podup.project=$2; not replacing it\"",
	];
	let mut rest = script.to_string();
	for message in messages {
		assert!(rest.contains(message));
		rest = rest.replace(message, "");
	}
	let rest = rest.replace("$((0$m))", "").replace("$((0$4))", "");
	let bytes = rest.as_bytes();
	for (at, _) in rest.match_indices('$') {
		if matches!(bytes.get(at + 1), Some(b'1' | b'2' | b'3' | b'4')) {
			assert!(
				at > 0 && matches!(bytes[at - 1], b'"' | b'=') && bytes.get(at + 2) == Some(&b'"'),
				"an unquoted parameter at in"
			);
		}
	}
}

/// systemd expands `$NAME` inside `ExecStart=`: every `$` of the script is
/// written as `$$`, so the shell receives `$1`.
#[test]
fn every_dollar_in_the_unit_is_doubled_for_systemd() {
	let lines = exec_lines_of(
		"services:\n  web:\n    image: img\n    secrets: [a]\nsecrets:\n  a:\n    file: ./one\n",
	);
	let line = &lines[0];
	assert!(line.contains("$$1") && line.contains("$$2") && line.contains("$$3"));
	assert!(!line.replace("$$", "").contains('$'));
}
