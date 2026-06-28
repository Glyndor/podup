//! Render service `secrets:` and `security_opt:` onto their Quadlet keys.

use indexmap::IndexMap;

use crate::compose::types::{SecretConfig, ServiceSecretRef};

use super::Section;

/// Sanitize one `Secret=` option-list field: drop control characters and the
/// `,`/`=` separators so a hostile compose value cannot inject extra options.
pub(super) fn secret_field(value: &str) -> String {
	value
		.chars()
		.filter(|c| !c.is_control() && *c != ',' && *c != '=')
		.collect()
}

/// Whether a top-level secret definition is an inline `content:`/`environment:`
/// source — the kind `up` materialises as a project-scoped native Podman secret.
/// `external:` wins (never created by podup) and a bare `file:`/empty def is a
/// bind/host source kept under its compose name.
pub(super) fn is_inline_secret(def: Option<&SecretConfig>) -> bool {
	def.is_some_and(|d| {
		d.external != Some(true) && (d.content.is_some() || d.environment.is_some())
	})
}

/// Resolve the `Secret=` source name for a service secret reference. An inline
/// secret resolves to the project-scoped name `{project}_secret_{name}` that
/// `up` creates (via `plan::scoped_name`), so generated units reference the same
/// secret `up` would; any other secret keeps its compose source name.
fn secret_source_name(
	project: &str,
	source: &str,
	secrets: &IndexMap<String, SecretConfig>,
) -> String {
	if is_inline_secret(secrets.get(source)) {
		format!("{project}_secret_{source}")
	} else {
		source.to_string()
	}
}

/// Render a service `secrets:` entry into a Quadlet `Secret=` value
/// (`name[,target=,uid=,gid=,mode=]`), resolving inline secrets to their
/// project-scoped name so the reference matches what `up` creates.
pub(super) fn render_secret(
	secret: &ServiceSecretRef,
	project: &str,
	secrets: &IndexMap<String, SecretConfig>,
) -> String {
	match secret {
		// Sanitize the short-form name too: `Secret=` is an option list, so a
		// `,`/`=` in the name would inject extra options (same guard as Long).
		ServiceSecretRef::Short(name) => secret_field(&secret_source_name(project, name, secrets)),
		ServiceSecretRef::Long {
			source,
			target,
			uid,
			gid,
			mode,
		} => {
			// `Secret=` is a comma-separated `key=value` option list, so a `,`
			// or `=` embedded in any field would inject extra options. Strip
			// those (and control chars) from each value at the boundary.
			let mut s = secret_field(&secret_source_name(project, source, secrets));
			if let Some(t) = target {
				s.push_str(&format!(",target={}", secret_field(t)));
			}
			if let Some(u) = uid {
				s.push_str(&format!(",uid={}", secret_field(u)));
			}
			if let Some(g) = gid {
				s.push_str(&format!(",gid={}", secret_field(g)));
			}
			if let Some(m) = mode {
				s.push_str(&format!(",mode={m:o}"));
			}
			s
		}
	}
}

/// Map a single compose `security_opt` entry onto the dedicated Quadlet key
/// where one exists; unrecognized entries are reported rather than dropped.
pub(super) fn map_security_opt(
	opt: &str,
	container: &mut Section,
	name: &str,
	warnings: &mut Vec<String>,
) {
	if let Some(rest) = opt.strip_prefix("no-new-privileges") {
		let val = rest.trim_start_matches([':', '=']);
		let enabled = val.is_empty() || val == "true";
		container.add("NoNewPrivileges", enabled.to_string());
	} else if let Some(profile) = opt.strip_prefix("seccomp=") {
		container.add("SeccompProfile", profile.to_string());
	} else if let Some(profile) = opt
		.strip_prefix("apparmor=")
		.or_else(|| opt.strip_prefix("apparmor:"))
	{
		// `AppArmor=` is not a recognised [Container] Quadlet key (Quadlet would
		// drop the whole unit at daemon-reload), so route it through PodmanArgs= as
		// `--security-opt apparmor=<profile>`, like the other escape-hatch flags.
		container.add("PodmanArgs", format!("--security-opt apparmor={profile}"));
	} else if let Some(label) = opt.strip_prefix("label=") {
		if label == "disable" {
			container.add("SecurityLabelDisable", "true".to_string());
		} else if label == "nested" {
			container.add("SecurityLabelNested", "true".to_string());
		} else if let Some(t) = label.strip_prefix("type:") {
			container.add("SecurityLabelType", t.to_string());
		} else if let Some(l) = label.strip_prefix("level:") {
			container.add("SecurityLabelLevel", l.to_string());
		} else if let Some(f) = label.strip_prefix("filetype:") {
			container.add("SecurityLabelFileType", f.to_string());
		} else {
			warnings.push(format!(
				"{name}: security_opt 'label={label}' has no Quadlet key and is skipped"
			));
		}
	} else if let Some(paths) = opt.strip_prefix("mask=") {
		container.add("Mask", paths.to_string());
	} else if let Some(paths) = opt.strip_prefix("unmask=") {
		container.add("Unmask", paths.to_string());
	} else {
		warnings.push(format!(
			"{name}: security_opt '{opt}' has no Quadlet mapping and is skipped"
		));
	}
}

#[cfg(test)]
mod tests {
	use super::{is_inline_secret, map_security_opt, render_secret, secret_field, Section};
	use crate::compose::types::{SecretConfig, ServiceSecretRef};
	use indexmap::IndexMap;

	/// Render a secret with no top-level definitions in scope (so no inline
	/// scoping applies).
	fn render_bare(secret: &ServiceSecretRef) -> String {
		render_secret(secret, "proj", &IndexMap::new())
	}

	/// Render a fresh `[Container]` section after applying one `security_opt`,
	/// returning `(rendered_body, warnings)`.
	fn map_one(opt: &str) -> (String, Vec<String>) {
		let mut container = Section::new("Container");
		let mut warnings = Vec::new();
		map_security_opt(opt, &mut container, "web", &mut warnings);
		(container.render(), warnings)
	}

	#[test]
	fn secret_field_strips_separators_and_controls() {
		assert_eq!(secret_field("a,b=c\nd"), "abcd");
		assert_eq!(secret_field("plain"), "plain");
	}

	#[test]
	fn render_secret_short_form_sanitizes_name() {
		// A short-form name with a `,`/`=` would inject extra `Secret=` options.
		let s = ServiceSecretRef::Short("tok,uid=0".into());
		assert_eq!(render_bare(&s), "tokuid0");
	}

	#[test]
	fn render_secret_emits_uid_gid_mode() {
		// uid/gid pass through `secret_field`; mode is rendered octal.
		let s = ServiceSecretRef::Long {
			source: "tok".into(),
			target: Some("/run/tok".into()),
			uid: Some("1000".into()),
			gid: Some("1000".into()),
			mode: Some(0o440),
		};
		assert_eq!(
			render_bare(&s),
			"tok,target=/run/tok,uid=1000,gid=1000,mode=440"
		);
	}

	#[test]
	fn render_secret_cannot_inject_extra_options() {
		// A hostile target tries to smuggle a second option via `,` and `=`.
		let s = ServiceSecretRef::Long {
			source: "tok".into(),
			target: Some("/run/x,uid=0".into()),
			uid: None,
			gid: None,
			mode: None,
		};
		let out = render_bare(&s);
		// The injected `,uid=0` must be flattened into the target value, not a
		// separate option: exactly one comma (the legitimate `,target=`).
		assert_eq!(out.matches(',').count(), 1);
		assert_eq!(out, "tok,target=/run/xuid0");
	}

	// --- map_security_opt: each recognized form maps to its dedicated key -----

	#[test]
	fn maps_no_new_privileges_bare_and_explicit() {
		let (body, warnings) = map_one("no-new-privileges");
		assert!(body.contains("NoNewPrivileges=true"));
		assert!(warnings.is_empty());

		assert!(map_one("no-new-privileges:false")
			.0
			.contains("NoNewPrivileges=false"));
		assert!(map_one("no-new-privileges=true")
			.0
			.contains("NoNewPrivileges=true"));
	}

	#[test]
	fn maps_seccomp_and_apparmor() {
		assert!(map_one("seccomp=/etc/seccomp.json")
			.0
			.contains("SeccompProfile=/etc/seccomp.json"));
		// `AppArmor=` is not a recognised Quadlet key, so both `apparmor=` and
		// `apparmor:` route through PodmanArgs= as a `--security-opt` flag.
		assert!(map_one("apparmor=docker-default")
			.0
			.contains("PodmanArgs=--security-opt apparmor=docker-default"));
		assert!(map_one("apparmor:unconfined")
			.0
			.contains("PodmanArgs=--security-opt apparmor=unconfined"));
	}

	#[test]
	fn inline_secret_resolves_to_project_scoped_name() {
		// An inline (`content:`) secret must reference the project-scoped name
		// `up` creates, not the bare compose name.
		let mut defs = IndexMap::new();
		defs.insert(
			"tok".to_string(),
			SecretConfig {
				content: Some("s3cr3t".into()),
				..Default::default()
			},
		);
		assert!(is_inline_secret(defs.get("tok")));
		let s = ServiceSecretRef::Short("tok".into());
		assert_eq!(render_secret(&s, "proj", &defs), "proj_secret_tok");

		// A file-based secret keeps its compose name (it is a host source, not a
		// project-scoped native secret).
		let mut file_defs = IndexMap::new();
		file_defs.insert(
			"tok".to_string(),
			SecretConfig {
				file: Some("./tok.txt".into()),
				..Default::default()
			},
		);
		assert!(!is_inline_secret(file_defs.get("tok")));
		assert_eq!(render_secret(&s, "proj", &file_defs), "tok");

		// An external secret is referenced by its existing name, never scoped.
		let mut ext_defs = IndexMap::new();
		ext_defs.insert(
			"tok".to_string(),
			SecretConfig {
				external: Some(true),
				content: Some("ignored".into()),
				..Default::default()
			},
		);
		assert!(!is_inline_secret(ext_defs.get("tok")));
		assert_eq!(render_secret(&s, "proj", &ext_defs), "tok");
	}

	#[test]
	fn maps_each_label_variant() {
		assert!(map_one("label=disable")
			.0
			.contains("SecurityLabelDisable=true"));
		assert!(map_one("label=nested")
			.0
			.contains("SecurityLabelNested=true"));
		assert!(map_one("label=type:container_t")
			.0
			.contains("SecurityLabelType=container_t"));
		assert!(map_one("label=level:s0:c1,c2")
			.0
			.contains("SecurityLabelLevel=s0:c1,c2"));
		assert!(map_one("label=filetype:tmp_t")
			.0
			.contains("SecurityLabelFileType=tmp_t"));
	}

	#[test]
	fn maps_mask_and_unmask() {
		assert!(map_one("mask=/proc/kcore").0.contains("Mask=/proc/kcore"));
		assert!(map_one("unmask=/sys/firmware")
			.0
			.contains("Unmask=/sys/firmware"));
	}

	#[test]
	fn unknown_label_warns_and_emits_no_key() {
		let (body, warnings) = map_one("label=bogus");
		// Only the section header — no key added.
		assert_eq!(body, "[Container]\n");
		assert_eq!(warnings.len(), 1);
		assert!(warnings[0].contains("label=bogus"));
	}

	#[test]
	fn unrecognized_opt_warns_and_emits_no_key() {
		let (body, warnings) = map_one("proc-opts=nosuid");
		assert_eq!(body, "[Container]\n");
		assert_eq!(warnings.len(), 1);
		assert!(warnings[0].contains("proc-opts=nosuid"));
	}
}
