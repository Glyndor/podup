//! Reading and writing the systemd user manager: the `systemctl`/`loginctl`
//! seam, the status report, and the pre-action advisories.

pub(super) mod guards;
pub(super) mod status;
pub(super) mod systemctl;
