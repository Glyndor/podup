//! `uid=` and `gid=` in the options of a `tmpfs:` entry.
//!
//! The Compose Specification lists them (`/data:mode=755,uid=1009,gid=1009`) and
//! Docker Compose gives the tmpfs that owner. Podman has no such option: `podman
//! run --tmpfs /t:uid=1000` is `unknown mount option "uid=1000"`. What it has is
//! `U`, which hands the mount to the user the container runs as. So the pair is
//! honoured when it names that very user, which `user:` states, and refused with
//! a message that says why in every other case, instead of a bare HTTP 500.

/// A value from the file with its control characters escaped, for a message.
fn clean(value: &str) -> std::borrow::Cow<'_, str> {
	crate::error::sanitize_name(value)
}

/// The options of a tmpfs entry with `uid=`/`gid=` turned into `U` when they
/// name the user the container runs as, or the reason they cannot be.
pub(crate) fn translate_owner(
	options: &[String],
	user: Option<&str>,
) -> Result<Vec<String>, String> {
	let mut rest = Vec::new();
	let (mut uid, mut gid) = (None, None);
	for option in options {
		match option.split_once('=') {
			Some(("uid", value)) => uid = Some(value),
			Some(("gid", value)) => gid = Some(value),
			_ => rest.push(option.clone()),
		}
	}
	if uid.is_none() && gid.is_none() {
		return Ok(options.to_vec());
	}
	let Some(user) = user else {
		return Err(format!(
			"Podman has no uid/gid option for a tmpfs; it can give the tmpfs to the user \
			 the container runs as, so set `user: \"{}:{}\"` on the service",
			clean(uid.unwrap_or("<uid>")),
			clean(gid.unwrap_or("<gid>"))
		));
	};
	let (user_uid, user_gid) = user
		.split_once(':')
		.map_or((user, None), |(u, g)| (u, Some(g)));
	let same = |asked: Option<&str>, has: Option<&str>| asked.is_none_or(|a| has == Some(a));
	if user_uid.parse::<u32>().is_err() || !same(uid, Some(user_uid)) || !same(gid, user_gid) {
		return Err(format!(
			"Podman has no uid/gid option for a tmpfs; it can give the tmpfs to the user the \
			 container runs as, and `user: {}` is not the owner asked for (uid={}, gid={}); \
			 use a numeric `user: \"<uid>:<gid>\"` equal to it",
			clean(user),
			clean(uid.unwrap_or("-")),
			clean(gid.unwrap_or("-"))
		));
	}
	rest.push("U".to_string());
	Ok(rest)
}

/// A whole `tmpfs:` entry (`/path:opt,opt`) with its `uid=`/`gid=` turned into
/// `U` by [`translate_owner`], for the writers that keep the entry as text.
pub(crate) fn translate_entry(entry: &str, user: Option<&str>) -> Result<String, String> {
	let (path, options) = entry.split_once(':').unwrap_or((entry, ""));
	let options: Vec<String> = options
		.split(',')
		.filter(|o| !o.is_empty())
		.map(str::to_string)
		.collect();
	let options = translate_owner(&options, user)?;
	Ok(if options.is_empty() {
		path.to_string()
	} else {
		format!("{path}:{}", options.join(","))
	})
}
