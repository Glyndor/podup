//! Stands in for `internal/audit/checks.rs`: the module the real file sits in.

pub mod check_fns {
	use super::super::Finding;

	/// Same body as `internal/audit/check_fns/mod.rs::finding`.
	pub fn finding(name: &str, check: &'static str, reason: &str) -> Finding {
		Finding {
			service: name.to_string(),
			check,
			reason: reason.to_string(),
		}
	}
}

#[path = "../../../internal/audit/check_sensitive_bind.rs"]
pub mod check_sensitive_bind;
