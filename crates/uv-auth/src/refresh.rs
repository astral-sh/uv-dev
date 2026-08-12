use std::path::Path;
use std::time::Duration;

use uv_fs::{LockedFile, LockedFileError, LockedFileMode};

/// Return whether an access token expires before the provider-specific safety window.
pub(crate) fn expires_within(expires_at: jiff::Timestamp, tolerance: Duration) -> bool {
    expires_at <= jiff::Timestamp::now() + tolerance
}

/// Serialize refresh-token rotation across uv processes.
///
/// Callers must reload token state after acquiring the lock because another process may have
/// already consumed a single-use refresh token.
pub(crate) async fn acquire_token_lock(
    path: &Path,
    description: &str,
) -> Result<LockedFile, LockedFileError> {
    LockedFile::acquire(path, LockedFileMode::Exclusive, description).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_windows_remain_provider_specific() {
        let expires_at = jiff::Timestamp::now() + Duration::from_secs(20);

        assert!(!expires_within(expires_at, Duration::from_secs(10)));
        assert!(expires_within(expires_at, Duration::from_secs(30)));
    }
}
