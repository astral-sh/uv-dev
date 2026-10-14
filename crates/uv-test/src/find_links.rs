//! A local HTTP server that serves a directory of files as a PEP 503 flat link page.
//!
//! Useful for testing `--find-links` with an HTTP URL.

use std::collections::BTreeMap;
use std::path::Path;

use indoc::formatdoc;
use wiremock::{Request, ResponseTemplate};

use crate::http_server::{FileData, HttpServer, content_type_for_filename};
use crate::vendor::{VendorArtifact, vendor_artifacts};

/// A running HTTP server that serves files from a directory as a flat links page.
pub struct FindLinksServer {
    server: HttpServer,
}

impl FindLinksServer {
    /// Start a server that serves all files in the given directory.
    pub fn new(directory: &Path) -> Self {
        let mut files = BTreeMap::new();

        for entry in fs_err::read_dir(directory).expect("failed to read find-links directory") {
            let entry = entry.expect("failed to read directory entry");
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(filename) = path.file_name().map(|n| n.to_string_lossy().to_string()) else {
                continue;
            };
            let bytes = fs_err::read(&path).expect("failed to read file");
            files.insert(filename, FileData::Bytes(bytes.into()));
        }
        Self::from_files(files)
    }

    /// Start a server that serves the pinned registry artifacts used by tests.
    pub fn vendor() -> Self {
        Self::from_vendor_artifacts(vendor_artifacts())
    }

    fn from_vendor_artifacts(artifacts: &'static [VendorArtifact]) -> Self {
        let files = artifacts
            .iter()
            .map(|artifact| (artifact.filename.to_string(), FileData::Vendor(artifact)))
            .collect();
        Self::from_files(files)
    }

    fn from_files(files: BTreeMap<String, FileData>) -> Self {
        let server = HttpServer::start(move |request, server_uri| {
            handle_request(request, server_uri, &files)
        });
        Self { server }
    }

    /// The base URL of the server (for use with `--find-links`).
    pub fn url(&self) -> &str {
        self.server.url()
    }
}

fn handle_request(
    request: &Request,
    server_uri: &str,
    files: &BTreeMap<String, FileData>,
) -> ResponseTemplate {
    let path = request.url.path();

    if path == "/" {
        let links = files
            .keys()
            .map(|filename| format!("<a href=\"{server_uri}/{filename}\">{filename}</a>"))
            .collect::<Vec<_>>()
            .join("\n");
        let html = formatdoc! {"
            <!DOCTYPE html>
            <html><body>
            {links}
            </body></html>
        "};
        return ResponseTemplate::new(200).set_body_raw(html, "text/html");
    }

    let filename = path.trim_start_matches('/');
    if let Some(file) = files.get(filename) {
        return match file.bytes() {
            Ok(bytes) => ResponseTemplate::new(200)
                .set_body_raw(bytes.to_vec(), content_type_for_filename(filename)),
            Err(error) => ResponseTemplate::new(500).set_body_string(format!("{error:#}")),
        };
    }

    ResponseTemplate::new(404)
}

#[cfg(test)]
mod tests {
    use super::FindLinksServer;
    use crate::vendor::VendorArtifact;

    #[tokio::test]
    async fn vendor_index_does_not_load_artifacts() -> anyhow::Result<()> {
        static ARTIFACTS: [VendorArtifact; 1] = [VendorArtifact::test_fixture()];
        let server = FindLinksServer::from_vendor_artifacts(&ARTIFACTS);
        let index = reqwest::Client::builder()
            .no_proxy()
            .build()?
            .get(server.url())
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        assert!(index.contains(ARTIFACTS[0].filename));
        assert!(!ARTIFACTS[0].is_loaded());
        Ok(())
    }
}
