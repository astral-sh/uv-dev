use anyhow::Result;
use uv_auth::Service;
use uv_distribution_types::IndexUrl;
use uv_pep508::VerbatimUrl;

pub(crate) mod dir;
pub(crate) mod helper;
pub(crate) mod login;
pub(crate) mod logout;
pub(crate) mod token;

/// Normalize a service for storing or removing credentials.
fn normalize_service(service: Service) -> Result<Service> {
    match IndexUrl::from(VerbatimUrl::from_url(service.url().clone())).root() {
        Some(root) => Ok(Service::try_from(root)?),
        None => Ok(service),
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::normalize_service;

    #[test]
    fn normalize_service_url() -> Result<()> {
        for (input, expected) in [
            ("https://example.com/simple", "https://example.com/"),
            (
                "https://example.com/root/+simple",
                "https://example.com/root",
            ),
            (
                "https://example.com/root/SiMpLe/",
                "https://example.com/root",
            ),
            (
                "https://example.com/root/simple/project",
                "https://example.com/root/simple/project",
            ),
            (
                "https://example.com/root/simple2",
                "https://example.com/root/simple2",
            ),
            (
                "https://user:p%40ss@example.com/root/simple?key=value#fragment",
                "https://user:p%40ss@example.com/root?key=value#fragment",
            ),
            ("http://localhost/simple", "http://localhost/"),
        ] {
            let service = normalize_service(input.parse()?)?;
            assert_eq!(service.url().as_str(), expected);
        }
        Ok(())
    }
}
