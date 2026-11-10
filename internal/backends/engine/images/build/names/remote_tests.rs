use super::*;

fn fold(context: &str, dockerfile: &str) -> RemoteDockerfile {
	fold_dockerfile_folder(context, dockerfile)
}

#[test]
fn the_folder_becomes_the_subfolder_of_a_context_with_a_reference() {
	let out = fold(
		"https://example.test/org/repo.git#main",
		"docker/Dockerfile",
	);
	assert_eq!(out.context, "https://example.test/org/repo.git#main:docker");
	assert_eq!(out.dockerfile, "Dockerfile");
	assert_eq!(out.moved_folder.as_deref(), Some("docker"));
}

#[test]
fn the_folder_is_added_below_an_existing_subfolder() {
	let out = fold(
		"https://example.test/org/repo.git#main:app",
		"docker/Dockerfile",
	);
	assert_eq!(
		out.context,
		"https://example.test/org/repo.git#main:app/docker"
	);
	assert_eq!(out.dockerfile, "Dockerfile");
}

#[test]
fn a_context_with_no_fragment_gets_an_empty_reference() {
	let out = fold("https://example.test/org/repo.git", "a/b/Dockerfile.prod");
	assert_eq!(out.context, "https://example.test/org/repo.git#:a/b");
	assert_eq!(out.dockerfile, "Dockerfile.prod");
}

#[test]
fn a_leading_dot_segment_is_ignored() {
	let out = fold("git@example.test:org/repo.git#v1", "./docker/Dockerfile");
	assert_eq!(out.context, "git@example.test:org/repo.git#v1:docker");
	assert_eq!(out.dockerfile, "Dockerfile");
}

#[test]
fn a_dockerfile_without_a_folder_is_left_alone() {
	let out = fold("https://example.test/org/repo.git#main", "Dockerfile");
	assert_eq!(out.context, "https://example.test/org/repo.git#main");
	assert_eq!(out.dockerfile, "Dockerfile");
	assert_eq!(out.moved_folder, None);
}

#[test]
fn a_path_that_leaves_the_context_is_left_alone() {
	for path in [
		"../Dockerfile",
		"docker/../../Dockerfile",
		"/etc/Dockerfile",
	] {
		let out = fold("https://example.test/org/repo.git#main", path);
		assert_eq!(out.dockerfile, path);
		assert_eq!(out.moved_folder, None);
	}
}

#[test]
fn a_context_that_is_not_git_is_left_alone() {
	let out = fold("https://example.test/context.tar.gz", "docker/Dockerfile");
	assert_eq!(out.context, "https://example.test/context.tar.gz");
	assert_eq!(out.dockerfile, "docker/Dockerfile");
	assert_eq!(out.moved_folder, None);
}
