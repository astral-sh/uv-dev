//! Default HTTP request policies shared by configuration and client construction.

use std::time::Duration;

/// Number of retries after a transient request failure.
pub const DEFAULT_RETRIES: u32 = 3;

/// The maximum time between two reads.
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// The maximum time to connect to a server.
///
/// This value is set lower to fail relatively quickly when the index is unreachable or down.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Total duration an upload may take.
///
/// reqwest does not support something like a read timeout for uploads, so we have to set a (large)
/// timeout on the entire upload.
pub const DEFAULT_READ_TIMEOUT_UPLOAD: Duration = Duration::from_mins(15);
