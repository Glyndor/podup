//! A closed transport for checking validation before any socket operation.

use super::Client;

impl Client {
	/// Refuse buffered requests in memory so preflight tests cannot open a socket.
	pub(crate) fn closed_for_preflight() -> Self {
		let client = Self::with_pool_size("unused", 1);
		client.pool.close();
		client
	}
}
