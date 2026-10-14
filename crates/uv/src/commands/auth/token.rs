use std::fmt::Write;

use anyhow::{Result, bail};
use uv_auth::{AuthBackend, Credentials, Service};
use uv_command_support::{ExitStatus, Printer};
use uv_preview::Preview;

use super::TOKEN_USERNAME;

/// Show the token that will be used for a service.
pub(crate) async fn token(
    service: Service,
    username: Option<String>,
    printer: Printer,
    preview: Preview,
) -> Result<ExitStatus> {
    let backend = AuthBackend::from_settings(preview).await?;
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

    let credentials = backend
        .get_credentials(url, Some(&username))
        .await?
        .ok_or_else(|| anyhow::anyhow!("Failed to fetch credentials for `{display_url}`"))?;

    let Some(password) = credentials.password() else {
        bail!(
            "No {} found for `{display_url}`",
            if username != TOKEN_USERNAME {
                "password"
            } else {
                "token"
            }
        );
    };

    writeln!(printer.stdout(), "{password}")?;
    Ok(ExitStatus::Success)
}
