//! The part of `internal/command_line/audit` that `check_sensitive_bind.rs` needs, so the
//! fuzz crate can compile that one file unchanged. The audit lives in the
//! `podup` binary, which a fuzz crate cannot link, and the file reaches only
//! two things outside itself: [`Finding`] and `check_fns::finding`. This is a
//! copy of their shape; the logic under test is the real file.

pub mod checks;

/// Same fields as `internal/command_line/audit/mod.rs::Finding`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
	pub service: String,
	pub check: &'static str,
	pub reason: String,
}
