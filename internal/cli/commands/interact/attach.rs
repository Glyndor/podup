//! Arguments of `attach`: which container is attached to, and the compatibility
//! flags other Compose tools accept on it.

use clap::Args;

#[derive(Args)]
pub(crate) struct AttachArgs {
	/// Service whose container to attach to.
	pub(crate) service: String,
	/// Index of the container when the service has multiple replicas (1-based).
	#[arg(long)]
	pub(crate) index: Option<u32>,
	/// Do not attach STDIN (accepted for compatibility; podup
	/// attaches output only).
	#[arg(long)]
	pub(crate) no_stdin: bool,
	/// Proxy received signals to the process (accepted for compatibility).
	///
	/// Takes an optional value so both spellings parse. Other Compose tools
	/// declare it as a bare flag, and demanding a value turned a script
	/// copied from one of them into a usage error, inside the very set of
	/// flags whose purpose is that such a script runs unchanged.
	#[arg(
		long,
		default_value_t = false,
		action = clap::ArgAction::Set,
		num_args = 0..=1,
		default_missing_value = "true",
	)]
	pub(crate) sig_proxy: bool,
	/// Override the detach key sequence (accepted for compatibility).
	#[arg(long)]
	pub(crate) detach_keys: Option<String>,
}
