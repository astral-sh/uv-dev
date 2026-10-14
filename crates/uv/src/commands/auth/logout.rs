use std::fmt::Write;

use anyhow::{Context, Result, bail};
use owo_colors::OwoColorize;

use uv_auth::{AuthBackend, Credentials, Service, Username};
use uv_command_support::{ExitStatus, Printer};
use uv_preview::Preview;

use super::TOKEN_USERNAME;

/// Logout from a service.
///
/// If no username is provided, defaults to `__token__`.
pub(crate) async fn logout(
    service: Service,
    username: Option<String>,
    printer: Printer,
    preview: Preview,
) -> Result<ExitStatus> {
    let backend = AuthBackend::from_settings(preview).await?;

    let service = super::service_root(service)?;
    let url = service.url();

    // Extract credentials from URL if present
    let url_credentials = Credentials::from_url(url)?;
    let url_username = url_credentials.as_ref().and_then(|c| c.username());

    let username = super::resolve_username(username, url_username)?
        .unwrap_or_else(|| TOKEN_USERNAME.to_owned());
    if username.is_empty() {
        bail!("Username cannot be empty");
    }

    let display_url = super::display_service(&service, &username);

    // TODO(zanieb): Consider exhaustively logging out from all backends
    match backend {
        AuthBackend::System(provider) => {
            provider
                .remove(url, &username)
                .await
                .with_context(|| format!("Unable to remove credentials for `{display_url}`"))?;
        }
        AuthBackend::TextStore(mut store, lock) => {
            if store
                .remove(service, Username::from(Some(username)))
                .is_none()
            {
                bail!("No matching entry found for `{display_url}`");
            }
            store
                .write(lock)
                .with_context(|| "Failed to persist changes to credentials after removal")?;
        }
    }

    writeln!(
        printer.stderr(),
        "Removed credentials for {}",
        display_url.bold().cyan()
    )?;

    Ok(ExitStatus::Success)
}
