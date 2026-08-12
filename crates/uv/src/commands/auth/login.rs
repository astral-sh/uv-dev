use std::fmt::Write;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use console::Term;
use owo_colors::OwoColorize;
use url::Url;
use uuid::Uuid;

use uv_auth::{
    AccessToken, AuthBackend, Credentials, PyxJwt, PyxOAuthTokens, PyxTokenStore, PyxTokens,
    Service, TextCredentialStore, is_default_pyx_domain, oidc,
};
use uv_client::{AuthIntegration, BaseClient, BaseClientBuilder};
use uv_distribution_types::IndexUrl;
use uv_pep508::VerbatimUrl;
use uv_preview::Preview;

use crate::commands::ExitStatus;
use crate::printer::Printer;

// We retry no more than this many times when polling for login status.
const STATUS_RETRY_LIMIT: u32 = 60;

/// Login to a service.
pub(crate) async fn login(
    service: Service,
    username: Option<String>,
    password: Option<String>,
    token: Option<String>,
    issuer: Option<Url>,
    client_id: Option<String>,
    scope: Option<String>,
    client_builder: BaseClientBuilder<'_>,
    printer: Printer,
    preview: Preview,
) -> Result<ExitStatus> {
    let pyx_store = PyxTokenStore::from_settings()?;
    if pyx_store.is_known_domain(service.url()) || is_default_pyx_domain(service.url()) {
        if username.is_some() {
            bail!("Cannot specify a username when logging in to pyx");
        }
        if password.is_some() {
            bail!("Cannot specify a password when logging in to pyx");
        }

        let client = client_builder
            .auth_integration(AuthIntegration::NoAuthMiddleware)
            .build()?;

        let access_token = pyx_login_with_browser(&pyx_store, &client, &printer).await?;
        let jwt = PyxJwt::decode(&access_token)?;

        if let Some(name) = jwt.name.as_deref() {
            writeln!(printer.stderr(), "Logged in to {}", name.bold().cyan())?;
        } else {
            writeln!(
                printer.stderr(),
                "Logged in to {}",
                pyx_store.api().bold().cyan()
            )?;
        }

        return Ok(ExitStatus::Success);
    }

    let backend = AuthBackend::from_settings(preview).await?;

    // If the URL includes a known index URL suffix, strip it
    // TODO(zanieb): Use a shared abstraction across `login` and `logout`?
    let url = service.url().clone();
    let (service, url) = match IndexUrl::from(VerbatimUrl::from_url(url.clone())).root() {
        Some(root) => (Service::try_from(root.clone())?, root),
        None => (service, url),
    };

    let url_credentials = Credentials::from_url(&url)?;

    if username.is_none() && password.is_none() && token.is_none() && url_credentials.is_none() {
        let client = client_builder
            .auth_integration(AuthIntegration::NoAuthMiddleware)
            .build()?;
        let discovery_url = issuer.as_ref().unwrap_or(service.url());

        if let Some(discovery) = oidc::discover(client.raw_client(), discovery_url).await? {
            if matches!(&backend, AuthBackend::System(_)) {
                bail!("The native authentication backend does not support bearer tokens");
            }

            return oidc_device_flow(
                &discovery,
                client.raw_client(),
                discovery_url,
                &service,
                backend,
                client_id.as_deref(),
                scope.as_deref(),
                printer,
            )
            .await;
        }

        if issuer.is_some() || client_id.is_some() || scope.is_some() {
            bail!(
                "OIDC discovery failed at `{discovery_url}`. Ensure the server exposes \
                 `/.well-known/openid-configuration`."
            );
        }
    }

    let url_username = url_credentials.as_ref().and_then(|c| c.username());
    let url_password = url_credentials.as_ref().and_then(|c| c.password());

    let username = match (username, url_username) {
        (Some(cli), Some(url)) => {
            bail!(
                "Cannot specify a username both via the URL and CLI; found `--username {cli}` and `{url}`"
            );
        }
        (Some(cli), None) => Some(cli),
        (None, Some(url)) => Some(url.to_string()),
        (None, None) => {
            // When using `--token`, we'll use a `__token__` placeholder username
            if token.is_some() {
                Some("__token__".to_string())
            } else {
                None
            }
        }
    };

    // Ensure that a username is not provided when using a token
    if token.is_some() {
        if let Some(username) = &username {
            if username != "__token__" {
                bail!("When using `--token`, a username cannot not be provided; found: {username}");
            }
        }
    }

    // Prompt for a username if not provided
    let username = if let Some(username) = username {
        username
    } else {
        let term = Term::stderr();
        if term.is_term() {
            let prompt = "username: ";
            uv_console::username(prompt, &term)?
        } else {
            bail!("No username provided; did you mean to provide `--username` or `--token`?");
        }
    };
    if username.is_empty() {
        bail!("Username cannot be empty");
    }

    let password = match (password, url_password, token) {
        (Some(_), Some(_), _) => {
            bail!("Cannot specify a password both via the URL and CLI");
        }
        (Some(_), None, Some(_)) => {
            bail!("Cannot specify a password via `--password` when using `--token`");
        }
        (None, Some(_), Some(_)) => {
            bail!("Cannot include a password in the URL when using `--token`")
        }
        (None, None, Some(value)) | (Some(value), None, None) if value == "-" => {
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            input.trim().to_string()
        }
        (Some(cli), None, None) => cli,
        (None, Some(url), None) => url.to_string(),
        (None, None, Some(token)) => token,
        (None, None, None) => {
            let term = Term::stderr();
            if term.is_term() {
                let prompt = "password: ";
                uv_console::password(prompt, &term)?
            } else {
                bail!("No password provided; did you mean to provide `--password` or `--token`?");
            }
        }
    };

    if password.is_empty() {
        bail!("Password cannot be empty");
    }

    let display_url = if username == "__token__" {
        url.without_credentials().to_string()
    } else {
        format!("{username}@{}", url.without_credentials())
    };

    // TODO(zanieb): Add support for other authentication schemes here, e.g., `Credentials::Bearer`
    let credentials = Credentials::basic(Some(username), Some(password));
    match backend {
        AuthBackend::System(provider) => {
            provider.store(&url, &credentials).await?;
        }
        AuthBackend::TextStore(mut store, _lock) => {
            store.insert(service.clone(), credentials);
            store.write(TextCredentialStore::default_file()?, _lock)?;
        }
    }

    writeln!(
        printer.stderr(),
        "Stored credentials for {}",
        display_url.bold().cyan()
    )?;
    Ok(ExitStatus::Success)
}

/// Log in via the [`PyxTokenStore`].
pub(crate) async fn pyx_login_with_browser(
    store: &PyxTokenStore,
    client: &BaseClient,
    printer: &Printer,
) -> Result<AccessToken> {
    // Generate a login code, like `67e55044-10b1-426f-9247-bb680e5fe0c8`.
    let cli_token = Uuid::new_v4();
    let url = {
        let mut url = store.api().clone();
        url.set_path(&format!("auth/cli/login/{cli_token}"));
        url
    };
    match open::that(url.as_ref()) {
        Ok(()) => {
            writeln!(printer.stderr(), "Logging in with {}", url.cyan().bold())?;
        }
        Err(..) => {
            writeln!(
                printer.stderr(),
                "Open the following URL in your browser: {}",
                url.cyan().bold()
            )?;
        }
    }

    // Poll the server for the login code.
    let url = {
        let mut url = store.api().clone();
        url.set_path(&format!("auth/cli/status/{cli_token}"));
        url
    };

    let mut retry = 0;
    let credentials = loop {
        let response = client
            .for_host(store.api())
            .get(Url::from(url.clone()))
            .send()
            .await?;
        match response.status() {
            // Retry on 404.
            reqwest::StatusCode::NOT_FOUND => {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                retry += 1;
            }
            // Parse the credentials on success.
            _ if response.status().is_success() => {
                let credentials = response.json::<PyxOAuthTokens>().await?;
                break Ok::<PyxTokens, anyhow::Error>(PyxTokens::OAuth(credentials));
            }
            // Fail on any other status code (like a 500).
            status => {
                break Err(anyhow::anyhow!("Failed to login with code `{status}`"));
            }
        }

        if retry >= STATUS_RETRY_LIMIT {
            break Err(anyhow::anyhow!(
                "Login session timed out after {STATUS_RETRY_LIMIT} seconds"
            ));
        }
    }?;

    store.write(&credentials).await?;

    Ok(AccessToken::from(credentials))
}

/// Perform the OAuth device authorization flow and persist the resulting bearer token.
async fn oidc_device_flow(
    discovery: &oidc::OidcDiscoveryDocument,
    client: &reqwest::Client,
    issuer: &Url,
    service: &Service,
    backend: AuthBackend,
    client_id: Option<&str>,
    scope: Option<&str>,
    printer: Printer,
) -> Result<ExitStatus> {
    let challenge = oidc::generate_pkce();
    let authorization =
        oidc::device_authorize(client, issuer, discovery, &challenge, client_id, scope).await?;

    writeln!(
        printer.stderr(),
        "Open {} and enter code: {}",
        authorization.verification_uri.cyan().bold(),
        authorization.user_code.bold(),
    )?;

    if Term::stderr().is_term() {
        let browser_url = authorization
            .verification_uri_complete
            .as_deref()
            .unwrap_or(&authorization.verification_uri);
        if open::that(browser_url).is_err() {
            writeln!(
                printer.stderr(),
                "Could not open the browser automatically; open the URL above manually."
            )?;
        }
    }

    let response = oidc::poll_for_token(
        client,
        issuer,
        discovery,
        &authorization,
        &challenge,
        client_id,
    )
    .await?;

    if response
        .token_type
        .as_deref()
        .is_some_and(|token_type| !token_type.eq_ignore_ascii_case("bearer"))
    {
        bail!("The device authorization server returned a non-bearer access token");
    }

    let Some(access_token) = response.access_token else {
        bail!("The device authorization server did not return an access token");
    };

    let session = oidc::session(issuer, discovery, client_id, scope, response.refresh_token)?;
    let credentials = if let Some(expires_in) = response.expires_in {
        let expires_at = jiff::Timestamp::now()
            .checked_add(Duration::from_secs(expires_in))
            .context("Invalid access token expiration returned by the authorization server")?;
        Credentials::bearer_with_expiration(access_token.into_bytes(), expires_at)
    } else {
        Credentials::bearer(access_token.into_bytes())
    };
    match backend {
        AuthBackend::System(provider) => {
            if !provider.store(service.url(), &credentials).await? {
                bail!("The native authentication backend does not support bearer tokens");
            }
        }
        AuthBackend::TextStore(mut store, lock) => {
            if let Some(session) = session {
                store.insert_oidc(service.clone(), credentials, session);
            } else {
                store.insert(service.clone(), credentials);
            }
            store.write(TextCredentialStore::default_file()?, lock)?;
        }
    }

    writeln!(
        printer.stderr(),
        "Stored credentials for {}",
        service
            .url()
            .without_credentials()
            .to_string()
            .bold()
            .cyan()
    )?;

    Ok(ExitStatus::Success)
}
