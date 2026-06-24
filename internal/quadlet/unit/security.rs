//! Render service `secrets:` and `security_opt:` onto their Quadlet keys.

use crate::compose::types::ServiceSecretRef;

use super::Section;

/// Sanitize one `Secret=` option-list field: drop control characters and the
/// `,`/`=` separators so a hostile compose value cannot inject extra options.
pub(super) fn secret_field(value: &str) -> String {
	value
		.chars()
		.filter(|c| !c.is_control() && *c != ',' && *c != '=')
		.collect()
}

/// Render a service `secrets:` entry into a Quadlet `Secret=` value
/// (`name[,target=,uid=,gid=,mode=]`).
pub(super) fn render_secret(secret: &ServiceSecretRef) -> String {
	match secret {
		// Sanitize the short-form name too: `Secret=` is an option list, so a
		// `,`/`=` in the name would inject extra options (same guard as Long).
		ServiceSecretRef::Short(name) => secret_field(name),
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
			let mut s = secret_field(source);
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
		// `AppArmor=` is a native [Container] key in current podman-systemd.unit(5);
		// emit it directly rather than as a raw `--security-opt apparmor=` flag.
		container.add("AppArmor", profile.to_string());
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
	use super::{map_security_opt, render_secret, secret_field, Section};
	use crate::compose::types::ServiceSecretRef;

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
		assert_eq!(render_secret(&s), "tokuid0");
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
			render_secret(&s),
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
		let out = render_secret(&s);
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
		// Both `apparmor=` and `apparmor:` map to the native AppArmor key.
		assert!(map_one("apparmor=docker-default")
			.0
			.contains("AppArmor=docker-default"));
		assert!(map_one("apparmor:unconfined")
			.0
			.contains("AppArmor=unconfined"));
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
