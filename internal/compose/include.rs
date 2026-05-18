//! `include:` directive — merging externally included compose files.
//!
//! Included files are merged into the parent: services, volumes, networks,
//! secrets, and configs from the included file are added only if the key does
//! not already exist in the parent (parent wins on conflict).

use super::types::ComposeFile;

/// Merge `other` into `target`.
///
/// Services / volumes / networks / secrets / configs from `other` are added;
/// existing entries in `target` win on conflict (parent file overrides included content).
pub(super) fn merge_compose_file(target: &mut ComposeFile, other: ComposeFile) {
    for (k, v) in other.services {
        target.services.entry(k).or_insert(v);
    }
    for (k, v) in other.volumes {
        target.volumes.entry(k).or_insert(v);
    }
    for (k, v) in other.networks {
        target.networks.entry(k).or_insert(v);
    }
    for (k, v) in other.secrets {
        target.secrets.entry(k).or_insert(v);
    }
    for (k, v) in other.configs {
        target.configs.entry(k).or_insert(v);
    }
}
