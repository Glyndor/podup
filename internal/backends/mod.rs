//! What podup does with a parsed project: run it on Podman through the engine, export it as Quadlet units, or install it as a boot-time systemd unit.

/// `podup autostart`: render and manage a rootless `systemctl --user` unit that
/// brings a compose stack up at boot (service mode).
pub mod autostart;
pub(crate) mod engine;
/// Quadlet export: translate a parsed compose file into Podman systemd units.
pub mod quadlet;

mod unit_directory;
