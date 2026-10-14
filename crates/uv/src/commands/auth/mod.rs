use anyhow::Result;

use uv_auth::Service;
use uv_distribution_types::IndexUrl;

pub(crate) mod dir;
pub(crate) mod helper;
pub(crate) mod login;
pub(crate) mod logout;
pub(crate) mod token;

/// The username used when an index token is stored as HTTP Basic credentials.
const TOKEN_USERNAME: &str = "__token__";

/// Use the index root as the credential service when the URL names a package-listing endpoint.
fn service_root(service: Service) -> Result<Service> {
    let Some(root) = IndexUrl::root_from_url(service.url()) else {
        return Ok(service);
    };
    Ok(Service::try_from(root)?)
}

/// Format a credential service without exposing embedded credentials or the token placeholder.
fn display_service(service: &Service, username: &str) -> String {
    let url = service.url().without_credentials();
    if username == TOKEN_USERNAME {
        url.to_string()
    } else {
        format!("{username}@{url}")
    }
}
