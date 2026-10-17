use anyhow::{Context, Result};
use reqwest_retry::policies::ExponentialBackoff;
use tracing::debug;

use uv_bin_install::{BinVersion, Binary, ResolvedVersion, find_matching_version};
use uv_client::BaseClient;

/// Resolve a requested version of an external binary.
pub(super) async fn resolve_version(
    binary: Binary,
    version: BinVersion,
    exclude_newer: Option<jiff::Timestamp>,
    client: &BaseClient,
    retry_policy: &ExponentialBackoff,
) -> Result<ResolvedVersion> {
    match version {
        BinVersion::Default => {
            let constraints = binary.default_constraints();
            let resolved = find_matching_version(
                binary,
                Some(&constraints),
                exclude_newer,
                client,
                retry_policy,
            )
            .await
            .with_context(|| {
                format!(
                    "Failed to find {binary} version matching default constraints: {constraints}"
                )
            })?;
            debug!(
                "Resolved `{binary}@{constraints}` to `{binary}=={}`",
                resolved.version
            );
            Ok(resolved)
        }
        BinVersion::Pinned(version) => {
            // Construct the exact download URL without fetching a versions manifest.
            if exclude_newer.is_some() {
                debug!("`--exclude-newer` is ignored for pinned version `{version}`");
            }
            Ok(ResolvedVersion::from_version(binary, version)?)
        }
        BinVersion::Latest => {
            let resolved = find_matching_version(binary, None, exclude_newer, client, retry_policy)
                .await
                .with_context(|| format!("Failed to find latest {binary} version"))?;
            debug!(
                "Resolved `{binary}@latest` to `{binary}=={}`",
                resolved.version
            );
            Ok(resolved)
        }
        BinVersion::Constraint(constraints) => {
            let resolved = find_matching_version(
                binary,
                Some(&constraints),
                exclude_newer,
                client,
                retry_policy,
            )
            .await
            .with_context(|| format!("Failed to find {binary} version matching: {constraints}"))?;
            debug!(
                "Resolved `{binary}@{constraints}` to `{binary}=={}`",
                resolved.version
            );
            Ok(resolved)
        }
    }
}
