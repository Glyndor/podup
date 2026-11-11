/// Whether a compose key may name a secret or a config that podup creates.
///
/// The key becomes part of a Podman secret name and a URL query parameter, so it
/// has to be a bounded, well-formed identifier. The Compose Specification keys
/// them `[a-zA-Z0-9._-]+`; podup also refuses a leading dot (a hidden or relative
/// name) and more than 128 characters. This is not the rule for a project name:
/// that one is lowercase only and has no dots, and reusing it here refused
/// `API_KEY` and `my.secret`, which Docker Compose accepts.
pub(super) fn is_valid_secret_name(name: &str) -> bool {
	!name.is_empty()
		&& name.len() <= 128
		&& !name.starts_with('.')
		&& name
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}
