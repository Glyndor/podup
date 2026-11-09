//! A Dockerfile that sits in a folder of a remote Git build context.
//!
//! Podman clones a Git context server-side and looks for the Dockerfile by
//! its file name alone, directly in the context folder: a `dockerfile` of
//! `docker/Dockerfile` is searched as `Dockerfile` at the top, or at the
//! `#ref:folder` the context names, and the request fails when it is not
//! there. The folder the Dockerfile sits in can only reach the daemon as part
//! of the context, so it moves from the `dockerfile` into the `#ref:folder`
//! part of the context URL.

/// The remote context and Dockerfile name to send to the daemon.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::backends::engine::images::build) struct RemoteDockerfile {
	/// The context URL, with the Dockerfile's folder added to its subfolder.
	pub(in crate::backends::engine::images::build) context: String,
	/// The Dockerfile's file name, with no folder.
	pub(in crate::backends::engine::images::build) dockerfile: String,
	/// The folder moved into the context, when there was one.
	pub(in crate::backends::engine::images::build) moved_folder: Option<String>,
}

/// True when the context is a Git repository, the only remote kind that can
/// name a subfolder.
fn is_git_context(context: &str) -> bool {
	let base = context.split('#').next().unwrap_or(context);
	context.contains('#')
		|| base.starts_with("git@")
		|| base.starts_with("git://")
		|| base.ends_with(".git")
}

/// The folder and file name of a Dockerfile path, when it is a plain relative
/// path with a folder in it. A path that leaves the context (`..`) or is
/// absolute cannot be expressed as a subfolder and is left alone.
fn split_folder(dockerfile: &str) -> Option<(String, &str)> {
	let parts: Vec<&str> = dockerfile
		.split('/')
		.filter(|part| !part.is_empty() && *part != ".")
		.collect();
	if dockerfile.starts_with('/') || parts.contains(&"..") {
		return None;
	}
	let (file, folders) = parts.split_last()?;
	if folders.is_empty() {
		return None;
	}
	Some((folders.join("/"), file))
}

/// Move the Dockerfile's folder into the context URL of a Git context.
///
/// Any other context, and a Dockerfile with no folder, comes back unchanged.
pub(in crate::backends::engine::images::build) fn fold_dockerfile_folder(
	context: &str,
	dockerfile: &str,
) -> RemoteDockerfile {
	let unchanged = || RemoteDockerfile {
		context: context.to_string(),
		dockerfile: dockerfile.to_string(),
		moved_folder: None,
	};
	if !is_git_context(context) {
		return unchanged();
	}
	let Some((folder, file)) = split_folder(dockerfile) else {
		return unchanged();
	};
	let (base, fragment) = context.split_once('#').unwrap_or((context, ""));
	let (reference, subfolder) = fragment.split_once(':').unwrap_or((fragment, ""));
	let subfolder = subfolder.trim_matches('/');
	let subfolder = if subfolder.is_empty() {
		folder.clone()
	} else {
		format!("{subfolder}/{folder}")
	};
	RemoteDockerfile {
		context: format!("{base}#{reference}:{subfolder}"),
		dockerfile: file.to_string(),
		moved_folder: Some(folder),
	}
}

#[cfg(test)]
#[path = "remote_tests.rs"]
mod tests;
