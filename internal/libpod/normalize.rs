//! Docker-compatible reference normalisation, the same shape the
//! docker-compat build handler applied through `NormalizeToDockerHub`
//! (Podman v5.7.0, `pkg/api/handlers/utils/images.go`).
//!
//! When podup talked to the docker compat layer every `t=` and
//! `/images/{}/tag?repo=&tag=` argument went through that helper. On
//! the libpod path the helper short-circuits (`IsLibpodRequest` makes
//! it return the input unchanged), so the canonical shape the user
//! used to get - a docker.io prefix on every unqualified name - is
//! gone, and a `podup build` that used to land `proj-app` as
//! `docker.io/library/proj-app:latest` now lands it as
//! `localhost/proj-app:latest`. Two copies of every image, and a
//! second copy of every `ps`/`images`/`events` row.
//!
//! Reproducing the helper here keeps the wire shape stable. Pure, so
//! the unit test drives the five inputs Podman itself distinguishes:
//! library with tag, org-scoped, registry-qualified (left alone),
//! digest, and a tag-less name.

/// Normalise `name` to its docker-compatible canonical form, the
/// way the docker compat handler did through
/// `NormalizeToDockerHub`. The rules, measured against Podman 5.7.0:
///
/// 1. A digest (`sha256:...`) is left alone. The upstream helper
///    rejected a digest from `reference.ParseNormalizedNamed` and
///    returned the candidate unchanged; we cannot match the same
///    parse path here and we do not need to: a digest is already in
///    its canonical form.
/// 2. A name whose first `/`-separated component contains `.` or
///    `:` or is `localhost` is a registry-qualified reference. The
///    upstream helper left those alone so `quay.io/x`,
///    `host:5000/x` and `localhost/x` keep their own registry. The
///    check sits behind a `had_slash` gate: a bare `app:latest` has
///    no `/`, the whole string is the library name and the `:` is
///    the tag separator, not a registry port.
/// 3. Otherwise the name is a library image when it has no `/`
///    (becomes `docker.io/library/<name>`) and an org-scoped image
///    when it has one (becomes `docker.io/<first>/<rest>`). The
///    tag, if any, rides through unchanged on either branch.
pub(crate) fn normalize_docker_reference(name: &str) -> String {
	if name.starts_with("sha256:") {
		return name.to_string();
	}
	let (first, had_slash, rest) = match name.split_once('/') {
		Some((f, r)) => (f, true, r),
		None => (name, false, ""),
	};
	let is_registry =
		had_slash && (first == "localhost" || first.contains('.') || first.contains(':'));
	if is_registry {
		return name.to_string();
	}
	if had_slash {
		format!("docker.io/{first}/{rest}")
	} else {
		format!("docker.io/library/{name}")
	}
}

#[cfg(test)]
#[path = "normalize_tests.rs"]
mod tests;
