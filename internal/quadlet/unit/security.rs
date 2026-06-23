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
	use super::{render_secret, secret_field};
	use crate::compose::types::ServiceSecretRef;

	#[test]
	fn secret_field_strips_separators_and_controls() {
		assert_eq!(secret_field("a,b=c\nd"), "abcd");
		assert_eq!(secret_field("plain"), "plain");
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
}
