//! Volume and secret/config mount helpers.
//!
//! [`Engine::create_volumes`] pre-creates named volumes before containers start.
//! [`build_binds`] and [`build_mounts`] convert `volumes:` entries to bollard's
//! bind-string and Mount-API formats respectively (tmpfs and volumes with
//! subpath/labels require the Mount API; simple bind/volume mounts use strings).
//! [`Engine::build_secret_binds`] and [`Engine::build_config_binds`] materialise
//! inline secrets/configs to a restricted temp directory and return bind strings.

use std::collections::HashMap;
use std::path::Path;

use bollard::models::{
    Mount, MountBindOptions, MountTmpfsOptions, MountType, MountVolumeOptions,
    MountVolumeOptionsDriverConfig, VolumeCreateRequest,
};
use tracing::info;

use crate::compose::types::{
    BindOptions, ComposeFile, ConfigConfig, SecretConfig, Service, ServiceConfigRef,
    ServiceSecretRef, VolumeMount, VolumeOptions, VolumeType,
};
use crate::error::{ComposeError, Result};

use super::Engine;

impl Engine {
    pub(super) async fn create_volumes(&self, file: &ComposeFile) -> Result<()> {
        for (name, config) in &file.volumes {
            let external = config.as_ref().and_then(|c| c.external).unwrap_or(false);
            if external {
                continue;
            }

            let volume_name = config
                .as_ref()
                .and_then(|c| c.name.as_deref())
                .unwrap_or(name);

            let mut labels: HashMap<String, String> = config
                .as_ref()
                .map(|c| c.labels.to_map())
                .unwrap_or_default();
            labels.insert("podup.project".to_string(), self.project.clone());

            let driver = config
                .as_ref()
                .and_then(|c| c.driver.clone())
                .unwrap_or_else(|| "local".into());

            let driver_opts: HashMap<String, String> = config
                .as_ref()
                .map(|c| c.driver_opts.clone())
                .unwrap_or_default();

            let options = VolumeCreateRequest {
                name: Some(volume_name.to_string()),
                driver: Some(driver.clone()),
                driver_opts: if driver_opts.is_empty() {
                    None
                } else {
                    Some(driver_opts)
                },
                labels: if labels.is_empty() {
                    None
                } else {
                    Some(labels)
                },
                ..Default::default()
            };

            match self.docker.create_volume(options).await {
                Ok(_) => info!("created volume {volume_name}"),
                Err(bollard::errors::Error::DockerResponseServerError {
                    status_code: 409, ..
                }) => {}
                Err(e) => return Err(ComposeError::Podman(e)),
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Free helpers (pub(super) for container.rs)
// ---------------------------------------------------------------------------

pub(crate) fn build_binds(service: &Service, base_dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for v in &service.volumes {
        match v {
            VolumeMount::Short(s) => out.push(s.clone()),
            VolumeMount::Long {
                volume_type,
                source,
                target,
                read_only,
                bind,
                volume,
                ..
            } => {
                if matches!(volume_type, VolumeType::Tmpfs) {
                    continue;
                }
                // Volumes with subpath/labels/driver_config go through the Mount API.
                if needs_mount_api(volume) {
                    continue;
                }
                let src = source.as_deref().unwrap_or("");

                if matches!(volume_type, VolumeType::Bind) {
                    if let Some(b) = bind {
                        if b.create_host_path.unwrap_or(false) && !src.is_empty() {
                            let abs = if Path::new(src).is_absolute() {
                                std::path::PathBuf::from(src)
                            } else {
                                base_dir.join(src)
                            };
                            if let Err(e) = std::fs::create_dir_all(&abs) {
                                tracing::warn!(
                                    "create_host_path: failed to create {}: {e}",
                                    abs.display()
                                );
                            }
                        }
                    }
                }

                let mut opts: Vec<String> = Vec::new();
                if read_only.unwrap_or(false) {
                    opts.push("ro".into());
                } else {
                    opts.push("rw".into());
                }
                if let Some(b) = bind {
                    extend_bind_opts(&mut opts, b);
                }
                if let Some(vol) = volume {
                    extend_volume_opts(&mut opts, vol);
                }
                out.push(format!("{src}:{target}:{}", opts.join(",")));
            }
        }
    }
    out
}

fn needs_mount_api(volume: &Option<VolumeOptions>) -> bool {
    volume
        .as_ref()
        .is_some_and(|v| v.subpath.is_some() || !v.labels.is_empty() || v.driver_config.is_some())
}

pub(crate) fn build_mounts(service: &Service) -> Vec<Mount> {
    let mut out = Vec::new();
    for v in &service.volumes {
        if let VolumeMount::Long {
            volume_type,
            source,
            target,
            read_only,
            bind,
            volume,
            tmpfs,
            consistency,
        } = v
        {
            if matches!(volume_type, VolumeType::Tmpfs) {
                // Tmpfs via Mount API.
                let tmpfs_options = tmpfs.as_ref().map(|t| MountTmpfsOptions {
                    size_bytes: t.size.map(|s| s as i64),
                    mode: t.mode.map(|m| m as i64),
                    options: None,
                });
                out.push(Mount {
                    target: Some(target.clone()),
                    source: source.clone(),
                    typ: Some(MountType::TMPFS),
                    read_only: *read_only,
                    consistency: consistency.clone(),
                    tmpfs_options,
                    ..Default::default()
                });
                continue;
            }
            if !needs_mount_api(volume) {
                continue;
            }
            let mount_type = match volume_type {
                VolumeType::Bind => MountType::BIND,
                VolumeType::Volume => MountType::VOLUME,
                VolumeType::Npipe => MountType::NPIPE,
                VolumeType::Cluster => MountType::CLUSTER,
                VolumeType::Tmpfs => unreachable!(),
            };
            let bind_options = bind.as_ref().map(|b| MountBindOptions {
                propagation: b.propagation.as_deref().and_then(|p| p.parse().ok()),
                ..Default::default()
            });
            let volume_options = volume.as_ref().map(|v| {
                let labels = if v.labels.is_empty() {
                    None
                } else {
                    Some(v.labels.to_map())
                };
                let driver_config =
                    v.driver_config
                        .as_ref()
                        .map(|dc| MountVolumeOptionsDriverConfig {
                            name: dc.name.clone(),
                            options: if dc.options.is_empty() {
                                None
                            } else {
                                Some(dc.options.clone())
                            },
                        });
                MountVolumeOptions {
                    no_copy: v.nocopy,
                    labels,
                    driver_config,
                    subpath: v.subpath.clone(),
                }
            });
            out.push(Mount {
                target: Some(target.clone()),
                source: source.clone(),
                typ: Some(mount_type),
                read_only: *read_only,
                consistency: consistency.clone(),
                bind_options,
                volume_options,
                ..Default::default()
            });
        }
    }
    out
}

fn extend_bind_opts(opts: &mut Vec<String>, b: &BindOptions) {
    if let Some(p) = &b.propagation {
        opts.push(p.clone());
    }
    if let Some(s) = &b.selinux {
        opts.push(s.clone());
    }
}

fn extend_volume_opts(opts: &mut Vec<String>, v: &VolumeOptions) {
    if v.nocopy.unwrap_or(false) {
        opts.push("nocopy".into());
    }
}

impl Engine {
    pub(super) fn build_secret_binds(
        &self,
        service: &Service,
        file: &ComposeFile,
    ) -> Result<Vec<String>> {
        let mut binds = Vec::new();
        for secret_ref in &service.secrets {
            let (name, target_override, ref_mode, ref_uid, ref_gid) = match secret_ref {
                ServiceSecretRef::Short(s) => (s.clone(), None, None, None, None),
                ServiceSecretRef::Long {
                    source,
                    target,
                    mode,
                    uid,
                    gid,
                } => (
                    source.clone(),
                    target.clone(),
                    *mode,
                    uid.clone(),
                    gid.clone(),
                ),
            };
            if let Some(config) = file.secrets.get(&name) {
                let target = target_override.unwrap_or_else(|| format!("/run/secrets/{name}"));
                match config {
                    SecretConfig {
                        file: Some(host_path),
                        ..
                    } => {
                        binds.push(format!("{host_path}:{target}:ro"));
                    }
                    SecretConfig {
                        content: Some(content),
                        ..
                    } => {
                        let path = self.materialize_inline_full(
                            "secrets",
                            &name,
                            content.as_bytes(),
                            ref_mode,
                            ref_uid.as_deref(),
                            ref_gid.as_deref(),
                        )?;
                        binds.push(format!("{}:{target}:ro", path.display()));
                    }
                    SecretConfig {
                        environment: Some(env_var),
                        ..
                    } => {
                        let value = std::env::var(env_var).unwrap_or_default();
                        let path = self.materialize_inline_full(
                            "secrets",
                            &name,
                            value.as_bytes(),
                            ref_mode,
                            ref_uid.as_deref(),
                            ref_gid.as_deref(),
                        )?;
                        binds.push(format!("{}:{target}:ro", path.display()));
                    }
                    SecretConfig {
                        external: Some(true),
                        ..
                    } => {
                        tracing::debug!("external secret {name} — relying on runtime injection");
                    }
                    _ => {}
                }
            }
        }
        Ok(binds)
    }

    pub(super) fn build_config_binds(
        &self,
        service: &Service,
        file: &ComposeFile,
    ) -> Result<Vec<String>> {
        let mut binds = Vec::new();
        for config_ref in &service.configs {
            let (name, target_override, ref_mode, ref_uid, ref_gid) = match config_ref {
                ServiceConfigRef::Short(s) => (s.clone(), None, None, None, None),
                ServiceConfigRef::Long {
                    source,
                    target,
                    mode,
                    uid,
                    gid,
                } => (
                    source.clone(),
                    target.clone(),
                    *mode,
                    uid.clone(),
                    gid.clone(),
                ),
            };
            if let Some(cfg) = file.configs.get(&name) {
                let target = target_override.unwrap_or_else(|| format!("/{name}"));
                match cfg {
                    ConfigConfig {
                        file: Some(host_path),
                        ..
                    } => {
                        binds.push(format!("{host_path}:{target}:ro"));
                    }
                    ConfigConfig {
                        content: Some(content),
                        ..
                    } => {
                        let path = self.materialize_inline_full(
                            "configs",
                            &name,
                            content.as_bytes(),
                            ref_mode,
                            ref_uid.as_deref(),
                            ref_gid.as_deref(),
                        )?;
                        binds.push(format!("{}:{target}:ro", path.display()));
                    }
                    ConfigConfig {
                        environment: Some(env_var),
                        ..
                    } => {
                        let value = std::env::var(env_var).unwrap_or_default();
                        let path = self.materialize_inline_full(
                            "configs",
                            &name,
                            value.as_bytes(),
                            ref_mode,
                            ref_uid.as_deref(),
                            ref_gid.as_deref(),
                        )?;
                        binds.push(format!("{}:{target}:ro", path.display()));
                    }
                    ConfigConfig {
                        external: Some(true),
                        ..
                    } => {
                        tracing::debug!("external config {name} — relying on runtime injection");
                    }
                    _ => {}
                }
            }
        }
        Ok(binds)
    }

    /// Write `content` to a per-project temp file and return its path.
    ///
    fn materialize_inline_full(
        &self,
        kind: &str,
        name: &str,
        content: &[u8],
        mode: Option<u32>,
        uid: Option<&str>,
        gid: Option<&str>,
    ) -> Result<std::path::PathBuf> {
        use std::io::Write;
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

        // Reject names that could escape the temp dir (path traversal).
        if std::path::Path::new(name)
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(ComposeError::Unsupported(format!(
                "{kind} name must not contain path separators or '..': {name}"
            )));
        }

        let dir = self.staging_dir()?.join(kind);

        // Create dir with 0o700 so only the owning user can list/enter it.
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .map_err(ComposeError::Io)?;

        let path = dir.join(name);

        // Create file with 0o600 atomically — no world-readable window before chmod.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .map_err(ComposeError::Io)?;
        file.write_all(content).map_err(ComposeError::Io)?;
        drop(file);

        if let Some(m) = mode {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(m))
                .map_err(ComposeError::Io)?;
        }

        // Best-effort chown — succeeds in rootful Podman, no-op in rootless.
        let uid_val: libc::uid_t = uid.and_then(|s| s.parse().ok()).unwrap_or(u32::MAX);
        let gid_val: libc::gid_t = gid.and_then(|s| s.parse().ok()).unwrap_or(u32::MAX);
        if uid_val != u32::MAX || gid_val != u32::MAX {
            use std::ffi::CString;
            use std::os::unix::ffi::OsStrExt;
            if let Ok(p) = CString::new(path.as_os_str().as_bytes()) {
                let rc = unsafe { libc::chown(p.as_ptr(), uid_val, gid_val) };
                if rc != 0 {
                    tracing::warn!(
                        "chown failed for {}: {}",
                        path.display(),
                        std::io::Error::last_os_error()
                    );
                }
            }
        }

        Ok(path)
    }

    /// Remove the per-project temp directory created by `materialize_inline`.
    pub(super) fn cleanup_temp_dir(&self) {
        if let Ok(dir) = self.staging_dir() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// Per-project staging directory under a verified per-user base.
    ///
    /// The project name is validated first so it can never traverse out of
    /// the base — this same path is later passed to `remove_dir_all`.
    fn staging_dir(&self) -> Result<std::path::PathBuf> {
        if !is_safe_project_name(&self.project) {
            return Err(ComposeError::Unsupported(format!(
                "project name must be ASCII alphanumeric/dash/underscore/dot \
                 and must not start with a dot: {}",
                self.project
            )));
        }
        Ok(staging_base()?.join(&self.project))
    }
}

/// Whether `name` is safe to use as a single path component and container
/// name prefix: non-empty, bounded, ASCII alphanumeric plus `-`/`_`/`.`,
/// not starting with a dot (rejects `.`, `..` and hidden directories).
fn is_safe_project_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Per-user staging base for inline secret/config content.
///
/// Prefers `$XDG_RUNTIME_DIR/podup` (per-user and 0700 by contract); falls
/// back to `temp_dir()/podup-<euid>`. The base sits in a world-writable
/// parent in the fallback case, so after creation it is verified to be a
/// real directory (not a symlink), owned by the current user, with no
/// group/other permission bits. Anything else aborts (fail closed) instead
/// of writing secret material under — or later deleting — a path another
/// local user may control.
fn staging_base() -> Result<std::path::PathBuf> {
    use std::os::unix::fs::DirBuilderExt;

    // SAFETY: geteuid takes no arguments, touches no memory and cannot fail.
    let euid = unsafe { libc::geteuid() };

    let base = match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) if Path::new(&dir).is_absolute() => std::path::PathBuf::from(dir).join("podup"),
        _ => std::env::temp_dir().join(format!("podup-{euid}")),
    };

    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&base)
        .map_err(ComposeError::Io)?;

    verify_private_dir(&base, euid)?;
    Ok(base)
}

/// Verify that `dir` is a non-symlink directory owned by `euid` with no
/// group/other permission bits.
fn verify_private_dir(dir: &Path, euid: u32) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let meta = std::fs::symlink_metadata(dir).map_err(ComposeError::Io)?;
    if !meta.is_dir() || meta.uid() != euid || meta.mode() & 0o077 != 0 {
        return Err(ComposeError::Unsupported(format!(
            "staging directory {} is not a private directory owned by the \
             current user — refusing to use it",
            dir.display()
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::build_binds;
    use crate::compose::types::{BindOptions, Service, VolumeMount, VolumeOptions, VolumeType};
    use std::path::Path;

    fn svc_with_volumes(vols: Vec<VolumeMount>) -> Service {
        Service {
            volumes: vols,
            ..Default::default()
        }
    }

    #[test]
    fn short_form_passthrough() {
        let svc = svc_with_volumes(vec![VolumeMount::Short("./data:/app/data".into())]);
        let binds = build_binds(&svc, Path::new("/base"));
        assert_eq!(binds, vec!["./data:/app/data"]);
    }

    #[test]
    fn long_form_bind_read_only() {
        let svc = svc_with_volumes(vec![VolumeMount::Long {
            volume_type: VolumeType::Bind,
            source: Some("/host/path".into()),
            target: "/container/path".into(),
            read_only: Some(true),
            bind: None,
            volume: None,
            tmpfs: None,
            consistency: None,
        }]);
        let binds = build_binds(&svc, Path::new("/base"));
        assert_eq!(binds.len(), 1);
        assert!(binds[0].contains("ro"));
        assert!(binds[0].contains("/host/path:/container/path"));
    }

    #[test]
    fn long_form_bind_with_propagation() {
        let svc = svc_with_volumes(vec![VolumeMount::Long {
            volume_type: VolumeType::Bind,
            source: Some("/host".into()),
            target: "/cont".into(),
            read_only: Some(false),
            bind: Some(BindOptions {
                propagation: Some("rshared".into()),
                create_host_path: None,
                selinux: None,
            }),
            volume: None,
            tmpfs: None,
            consistency: None,
        }]);
        let binds = build_binds(&svc, Path::new("/base"));
        assert!(binds[0].contains("rshared"));
    }

    #[test]
    fn long_form_volume_nocopy() {
        let svc = svc_with_volumes(vec![VolumeMount::Long {
            volume_type: VolumeType::Volume,
            source: Some("myvolume".into()),
            target: "/data".into(),
            read_only: None,
            bind: None,
            volume: Some(VolumeOptions {
                nocopy: Some(true),
                ..Default::default()
            }),
            tmpfs: None,
            consistency: None,
        }]);
        let binds = build_binds(&svc, Path::new("/base"));
        assert!(binds[0].contains("nocopy"));
    }

    #[test]
    fn tmpfs_type_excluded_from_binds() {
        let svc = svc_with_volumes(vec![VolumeMount::Long {
            volume_type: VolumeType::Tmpfs,
            source: None,
            target: "/run".into(),
            read_only: None,
            bind: None,
            volume: None,
            tmpfs: None,
            consistency: None,
        }]);
        let binds = build_binds(&svc, Path::new("/base"));
        assert!(binds.is_empty());
    }
}

#[cfg(test)]
mod staging_tests {
    use super::{is_safe_project_name, verify_private_dir};
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn safe_project_names_accepted() {
        for name in ["web", "my-app", "my_app", "app.v2", "A1"] {
            assert!(is_safe_project_name(name), "{name:?} must be accepted");
        }
    }

    #[test]
    fn unsafe_project_names_rejected() {
        let long = "a".repeat(129);
        for name in [
            "",
            ".",
            "..",
            ".hidden",
            "a/b",
            "../x",
            "a b",
            "a\0b",
            long.as_str(),
        ] {
            assert!(!is_safe_project_name(name), "{name:?} must be rejected");
        }
    }

    #[test]
    fn private_dir_accepted() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("chmod");
        // SAFETY: geteuid takes no arguments, touches no memory and cannot fail.
        let euid = unsafe { libc::geteuid() };
        assert!(verify_private_dir(dir.path(), euid).is_ok());
    }

    #[test]
    fn group_accessible_dir_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o750))
            .expect("chmod");
        // SAFETY: geteuid takes no arguments, touches no memory and cannot fail.
        let euid = unsafe { libc::geteuid() };
        assert!(verify_private_dir(dir.path(), euid).is_err());
    }

    #[test]
    fn foreign_owner_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("chmod");
        // SAFETY: geteuid takes no arguments, touches no memory and cannot fail.
        let other = unsafe { libc::geteuid() } + 1;
        assert!(verify_private_dir(dir.path(), other).is_err());
    }

    #[test]
    fn symlink_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("real");
        let link = dir.path().join("link");
        std::fs::create_dir(&target).expect("mkdir");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).expect("chmod");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        // SAFETY: geteuid takes no arguments, touches no memory and cannot fail.
        let euid = unsafe { libc::geteuid() };
        assert!(verify_private_dir(&link, euid).is_err());
    }

    #[test]
    fn regular_file_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("file");
        std::fs::write(&file, b"x").expect("write");
        // SAFETY: geteuid takes no arguments, touches no memory and cannot fail.
        let euid = unsafe { libc::geteuid() };
        assert!(verify_private_dir(&file, euid).is_err());
    }
}
