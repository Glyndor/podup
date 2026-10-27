//! Keeping the anonymous volumes of the container a recreate replaces.
//!
//! A container that declares a volume without a source (`volumes: [/data]`)
//! gets an anonymous volume: one the runtime names, that the compose file
//! never mentions and that no `volumes:` entry can point at. The image's own
//! `VOLUME` directives produce the same thing at their own paths.
//!
//! Recreating the container used to drop them: the replacement's spec went
//! out with an empty name, the runtime minted a fresh volume, and the data
//! the previous container had written stayed behind in a volume nothing
//! pointed at any more. `-V/--renew-anon-volumes` is the way out of that, so
//! it is the only thing that may throw the old volume away; without it the
//! replacement inherits the volume its predecessor had.
//!
//! The inheritance is by destination, the same rule docker compose v2 uses
//! (`buildContainerMountOptions`): the volume mounted at `/data` is the one
//! the replacement gets at `/data`. A volume the compose file names is not
//! inherited — the file's own name wins — and a mount the compose file
//! dropped is not brought back.

use std::collections::{HashMap, HashSet};

use crate::error::{ComposeError, Result};
use crate::libpod::types::container::{Mount, NamedVolume};
use crate::libpod::urlencoded;
use crate::libpod::API_PREFIX;

use crate::engine::Engine;

impl Engine {
	/// Give the container that is about to replace `container_name` the
	/// anonymous volumes that container has.
	///
	/// `named_volumes` is the replacement's own volume list and is completed
	/// in place: an entry the compose file declared without a source takes
	/// the name of the volume its predecessor had at the same destination,
	/// and a path the image declares a `VOLUME` directive for gets the
	/// predecessor's volume added as an entry of its own.
	///
	/// `mounts` is the replacement's bind/tmpfs list, read to know which
	/// destinations already carry something else.
	pub(super) async fn inherit_anonymous_volumes(
		&self,
		container_name: &str,
		image: &str,
		named_volumes: &mut Vec<NamedVolume>,
		mounts: &[Mount],
	) -> Result<()> {
		let old = self.container_volumes(container_name).await?;
		if old.is_empty() {
			return Ok(());
		}
		// The volumes the compose file declares without a source: the same
		// destination gets the same volume.
		for nv in named_volumes.iter_mut() {
			if nv.name.is_empty() {
				if let Some(volume) = old.get(&nv.dest) {
					nv.name = volume.name.clone();
				}
			}
		}
		// The volumes the image declares with a `VOLUME` directive. The
		// runtime mounts an anonymous volume at each of those paths unless
		// the spec covers the destination, so the predecessor's volume is
		// inherited the same way. A path the compose file mounts itself is
		// already covered and is left alone.
		let covered: HashSet<String> = named_volumes
			.iter()
			.map(|nv| nv.dest.clone())
			.chain(mounts.iter().map(|m| m.destination.clone()))
			.collect();
		let uncovered: Vec<_> = old
			.iter()
			.filter(|(destination, _)| !covered.contains(*destination))
			.collect();
		if uncovered.is_empty() {
			return Ok(());
		}
		let image_volumes = self.image_volume_directives(image).await?;
		for (destination, volume) in uncovered {
			if !image_volumes.contains(destination) {
				continue;
			}
			named_volumes.push(NamedVolume {
				name: volume.name.clone(),
				dest: destination.clone(),
				options: if volume.rw {
					Vec::new()
				} else {
					vec!["ro".to_string()]
				},
				sub_path: None,
			});
		}
		Ok(())
	}

	/// The volume mounts an existing container has, keyed by the
	/// in-container path they are attached to.
	///
	/// A container that does not exist has nothing to inherit, so the
	/// "no such container" answer is an empty map and not an error.
	async fn container_volumes(
		&self,
		container_name: &str,
	) -> Result<HashMap<String, crate::libpod::types::container::ContainerMount>> {
		let path = format!(
			"{API_PREFIX}/containers/{}/json",
			urlencoded(container_name)
		);
		match self
			.client
			.get_json::<crate::libpod::types::container::ContainerInspect>(&path)
			.await
		{
			Ok(inspect) => Ok(inspect.volumes_by_destination()),
			Err(e) if e.is_status(404) => Ok(HashMap::new()),
			Err(e) => Err(ComposeError::Podman(e)),
		}
	}

	/// The paths the image declares a `VOLUME` directive for.
	///
	/// An image that is not there has no volume directives, so the "no such
	/// image" answer is an empty set and not an error; any other failure is
	/// the caller's to hear about.
	async fn image_volume_directives(&self, image: &str) -> Result<HashSet<String>> {
		let path = format!("{API_PREFIX}/images/{}/json", urlencoded(image));
		match self
			.client
			.get_json::<crate::libpod::types::image::ImageInspect>(&path)
			.await
		{
			Ok(inspect) => Ok(inspect
				.config
				.and_then(|config| config.volumes)
				.map(|volumes| volumes.into_keys().collect())
				.unwrap_or_default()),
			Err(e) if e.is_status(404) => Ok(HashSet::new()),
			Err(e) => Err(ComposeError::Podman(e)),
		}
	}
}
