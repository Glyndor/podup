//! The connection under the client: the per-socket pool, the byte stream and what a failed connect looks like, and the streaming response body with its connection state.

pub(in crate::connection::libpod::client) mod pool;
pub(in crate::connection::libpod::client) mod stream;
pub(in crate::connection::libpod::client) mod transport;
