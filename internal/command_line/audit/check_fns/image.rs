use podup::compose::types::{ComposeFile, Service};

use super::{finding, Finding};

/// `image:` is unpinned: no tag (defaults to `latest`), tag is `latest`,
/// or `latest` is not pinned by a digest. An `@sha256:` digest counts as
/// pinned even when a tag is also present.
///
/// A `build:` service can still pull when its image is missing locally and
/// the pull policy asks for one, so a blanket skip on `build:` would hide
/// a real registry reference. Only skip when the policy itself forbids
/// the fetch: `pull_policy: build` or `pull_policy: never`. In every
/// other case where `build:` is present the finding still fires, but
/// names the operator-actionable fix (set `pull_policy: build`) instead
/// of asking for a digest that no registry can serve.
pub fn check_unpinned_image(name: &str, service: &Service, _file: &ComposeFile) -> Vec<Finding> {
	let Some(reference) = service.image.as_deref() else {
		// A `build:` service without an `image:` is built from source and
		// has no registry reference to pin; out of scope for this check.
		return Vec::new();
	};
	if reference.contains('@') {
		// Any digest (sha256: or otherwise) anchors the tag, so the service
		// is pinned regardless of the tag's value.
		return Vec::new();
	}
	let last_colon = reference.rfind(':').unwrap_or(0);
	let last_slash = reference.rfind('/').unwrap_or(0);
	// The tag/separator sits after the last colon not preceded by a slash;
	// an image with no tag stops there.
	let has_tag = last_colon > last_slash;
	let needs_finding = !has_tag || &reference[last_colon + 1..] == "latest";
	if !needs_finding {
		return Vec::new();
	}
	if service.build.is_some()
		&& matches!(
			service.pull_policy.as_deref(),
			Some("build") | Some("never")
		) {
		// Skip here: the operator's `pull_policy` already commits to a
		// local-only run, so the check would duplicate a decision the
		// policy has locked in and have nothing actionable to add.
		return Vec::new();
	}
	if service.build.is_some() {
		// Use a different message because a digest can only anchor a tag
		// that exists in some registry; for a locally built image the
		// actionable fix is the pull policy, not a pin.
		return vec![finding(
			name,
			"unpinned_image",
			&format!(
				"image: {reference} is built here, so a digest cannot pin it; set pull_policy: build to keep it from being pulled"
			),
		)];
	}
	let msg = if !has_tag {
		format!("image: {reference} has no tag; defaults to :latest")
	} else {
		format!("image: {reference} pins to :latest, which moves under you")
	};
	vec![finding(name, "unpinned_image", &msg)]
}
