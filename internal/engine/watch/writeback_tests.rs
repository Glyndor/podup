use super::{effective_mounts, writes_back, EffectiveMount};
use crate::compose::types::Service;
use std::path::{Path, PathBuf};

fn mount(target: &str, source: &str, read_only: bool) -> EffectiveMount {
	EffectiveMount {
		target: target.into(),
		bind_source: Some(PathBuf::from(source)),
		read_only,
	}
}

fn non_bind(target: &str) -> EffectiveMount {
	EffectiveMount {
		target: target.into(),
		bind_source: None,
		read_only: false,
	}
}

fn assert_some_host(
	mounts: &[EffectiveMount],
	container_path: &str,
	watched: &str,
	expected: &str,
) {
	let got = writes_back(mounts, container_path, Path::new(watched));
	assert_eq!(
		got.as_deref().map(PathBuf::from),
		Some(PathBuf::from(expected)),
		"writes_back({container_path:?}, watched={watched:?}) = {got:?}, expected Some({expected:?})"
	);
}

fn assert_none(mounts: &[EffectiveMount], container_path: &str, watched: &str) {
	let got = writes_back(mounts, container_path, Path::new(watched));
	assert_eq!(
		got, None,
		"writes_back({container_path:?}, watched={watched:?}) = {got:?}, expected None"
	);
}

// 1. Identity ------------------------------------------------------------

#[test]
fn identity_bind_writes_back_into_its_own_source() {
	let mounts = vec![mount("/app", "/p/src", false)];
	assert_some_host(&mounts, "/app/f.txt", "/p/src", "/p/src/f.txt");
	assert_some_host(&mounts, "/app/sub/x", "/p/src", "/p/src/sub/x");
	assert_some_host(&mounts, "/app", "/p/src", "/p/src");
}

// 2. More specific named volume (carve-out) -------------------------------

#[test]
fn more_specific_volume_carve_out_short_circuits_the_bind() {
	let mounts = vec![mount("/app", "/p/src", false), non_bind("/app/cache")];
	// The carve-out wins for paths that land in /app/cache.
	assert_none(&mounts, "/app/cache/f", "/p/src");
	assert_none(&mounts, "/app/cache", "/p/src");
	// The bind is still the write-back target for sibling paths.
	assert_some_host(&mounts, "/app/other", "/p/src", "/p/src/other");
}

// 3. Shifted: rule path above bind source --------------------------------

#[test]
fn shifted_target_subpath_is_joined_under_the_bind_source() {
	let mounts = vec![mount("/app", "/p/src", false)];
	// Rule watches /p (rule target /app). Container /app/src/f lands at /p/src/src/f.
	assert_some_host(&mounts, "/app/src/f", "/p", "/p/src/src/f");
	assert_some_host(&mounts, "/app/a.txt", "/p", "/p/src/a.txt");
}

// 4. Bind target inside the rule destination ------------------------------

#[test]
fn bind_target_inside_rule_destination_writes_back_under_watched() {
	let mounts = vec![mount("/src", "/p/src", false)];
	// Rule watches /p, target /, bind is /src -> /p/src.
	assert_some_host(&mounts, "/src/f", "/p", "/p/src/f");
	// A path outside the bind stays out: the rule does not own /a.txt.
	assert_none(&mounts, "/a.txt", "/p");
}

// 5. Read-only bind -------------------------------------------------------

#[test]
fn read_only_bind_does_not_write_back() {
	let mounts = vec![mount("/app", "/p/src", true)];
	assert_none(&mounts, "/app/f", "/p/src");
	assert_none(&mounts, "/app", "/p/src");
}

// 6. Unrelated bind -------------------------------------------------------

#[test]
fn unrelated_bind_does_not_write_back() {
	let mounts = vec![mount("/app", "/p/other", false)];
	assert_none(&mounts, "/app/f", "/p/src");
}

// 7. Component-wise prefixes ---------------------------------------------

#[test]
fn component_wise_prefixes_do_not_cross_a_sibling() {
	let mounts = vec![mount("/app", "/p/src", false)];
	// /application is not /app: a different second component.
	assert_none(&mounts, "/application/f", "/p/src");
	// /p/src-two is not /p/src: a sibling component, not a prefix.
	assert_none(&mounts, "/app/x", "/p/src-two");
}

// 8. Tmpfs over the bind --------------------------------------------------

#[test]
fn tmpfs_over_the_bind_short_circuits_the_write_back() {
	let mounts = vec![mount("/app", "/p/src", false), non_bind("/app/tmp")];
	assert_none(&mounts, "/app/tmp/x", "/p/src");
	assert_none(&mounts, "/app/tmp", "/p/src");
	// Sibling paths still write back through the bind.
	assert_some_host(&mounts, "/app/other", "/p/src", "/p/src/other");
}

// Path outside any mount --------------------------------------------------

#[test]
fn no_matching_mount_returns_none() {
	let mounts = vec![mount("/app", "/p/src", false)];
	assert_none(&mounts, "/srv/f", "/p/src");
}

// Bind source outside watched root: the host path the copy would land on
// is not under the watched root, so the copy does not loop.

#[test]
fn bind_source_outside_watched_root_is_not_a_write_back() {
	let mounts = vec![mount("/app", "/p/elsewhere", false)];
	assert_none(&mounts, "/app/f", "/p/src");
}

// Longest prefix: equal-length tie, the iteration order decides, but the
// decision is still "no bind matches" when neither has a source. Pin the
// order-independent behaviour through an explicit check.

#[test]
fn longest_prefix_picks_the_deeper_carve_out() {
	// Three mounts at /app, /app/cache, /app/cache/inner. The deepest
	// non-bind one wins for paths inside /app/cache/inner.
	let mounts = vec![
		mount("/app", "/p/src", false),
		non_bind("/app/cache"),
		non_bind("/app/cache/inner"),
	];
	assert_none(&mounts, "/app/cache/inner/x", "/p/src");
	assert_none(&mounts, "/app/cache/x", "/p/src");
	assert_some_host(&mounts, "/app/x", "/p/src", "/p/src/x");
}

// effective_mounts: a real `Service` with three entries (one bind, one
// named volume, one tmpfs) produces three EffectiveMounts, the bind
// canonicalised against the tempdir base.

#[test]
fn effective_mounts_from_a_real_service() {
	let dir = tempfile::tempdir().unwrap();
	let base = dir.path();
	let src_dir = base.join("src");
	std::fs::create_dir(&src_dir).unwrap();
	let yaml =
		"image: x\nvolumes:\n  - \"./src:/app\"\n  - \"cache:/app/cache\"\ntmpfs:\n  - \"/t\"\n"
			.to_string();
	let svc: Service = serde_yaml::from_str(&yaml).unwrap();
	let mounts = effective_mounts(&svc, base);
	assert_eq!(mounts.len(), 3, "expected three mounts, got {mounts:?}");
	// Declaration order is volumes first, then tmpfs.
	assert_eq!(mounts[0].target, "/app");
	// The bind source is canonicalized: on macOS the tempdir lives under
	// /private/var, and on Windows the canonical form is a verbatim path.
	let src_canon = std::fs::canonicalize(&src_dir).unwrap();
	assert_eq!(mounts[0].bind_source.as_deref(), Some(src_canon.as_path()));
	assert!(!mounts[0].read_only);
	assert_eq!(mounts[1].target, "/app/cache");
	assert!(
		mounts[1].bind_source.is_none(),
		"named volume must not carry a bind source"
	);
	assert_eq!(mounts[2].target, "/t");
	assert!(
		mounts[2].bind_source.is_none(),
		"tmpfs must not carry a bind source"
	);
}

// effective_mounts: a read-only bind carries its read-only flag through.

#[test]
fn effective_mounts_preserves_read_only_flag() {
	let svc: Service = serde_yaml::from_str("image: x\nvolumes:\n  - \"./src:/app:ro\"\n").unwrap();
	let mounts = effective_mounts(&svc, Path::new("/base"));
	assert_eq!(mounts.len(), 1);
	assert!(mounts[0].read_only, "`:ro` must surface as read_only");
}

#[test]
fn a_bind_at_the_container_root_covers_every_path() {
	let mounts = vec![EffectiveMount {
		target: "/".to_string(),
		bind_source: Some(PathBuf::from("/p")),
		read_only: false,
	}];
	assert_eq!(
		writes_back(&mounts, "/src/f", Path::new("/p/src")),
		Some(PathBuf::from("/p/src/f"))
	);
	assert_eq!(writes_back(&mounts, "/other/f", Path::new("/p/src")), None);
}

// Normalisation: `.` and `..` in the container path collapse before the
// longest-prefix walk runs. A bind at `/app` with a `/app/cache` volume
// carve-out must recognise `/app/./cache/f` as inside the carve-out, not
// under the bind. A path that climbs one component must still land on
// the right host file.

#[test]
fn writes_back_drops_dot_components() {
	let mounts = vec![mount("/app", "/p/src", false), non_bind("/app/cache")];
	assert_none(&mounts, "/app/./cache/f", "/p/src");
}

#[test]
fn writes_back_resolves_dotdot_against_the_container_path() {
	let mounts = vec![mount("/app", "/p/src", false)];
	// /app/x/../f collapses to /app/f, which is under the bind.
	assert_some_host(&mounts, "/app/x/../f", "/p/src", "/p/src/f");
}

#[test]
fn writes_back_treats_trailing_slash_mount_target_as_no_slash() {
	let mounts = vec![EffectiveMount {
		target: "/app/".to_string(),
		bind_source: Some(PathBuf::from("/p/src")),
		read_only: false,
	}];
	assert_some_host(&mounts, "/app/f", "/p/src", "/p/src/f");
}
