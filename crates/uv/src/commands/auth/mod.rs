use anyhow::Result;

use uv_auth::Service;
use uv_distribution_types::IndexUrl;

pub(crate) mod dir;
pub(crate) mod helper;
pub(crate) mod login;
pub(crate) mod logout;
pub(crate) mod token;

/// Use the index root as the credential service when the URL names a package-listing endpoint.
fn service_root(service: Service) -> Result<Service> {
    let Some(root) = IndexUrl::root_from_url(service.url()) else {
        return Ok(service);
    };
    Ok(Service::try_from(root)?)
}
