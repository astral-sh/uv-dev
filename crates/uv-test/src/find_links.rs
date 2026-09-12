//! A local HTTP server that serves a directory of files as a PEP 503 flat link page.
//!
//! Useful for testing `--find-links` with an HTTP URL.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use wiremock::{Request, ResponseTemplate};

use crate::http_server::{HttpServer, content_type_for_filename};
use crate::vendor::{VendorArtifact, vendor_artifacts};

enum FileData {
    Bytes(Arc<[u8]>),
    Vendor(&'static VendorArtifact),
}

impl FileData {
    fn bytes(&self) -> anyhow::Result<Arc<[u8]>> {
        match self {
            Self::Bytes(bytes) => Ok(Arc::clone(bytes)),
            Self::Vendor(artifact) => artifact.bytes(),
        }
    }
}

/// A running HTTP server that serves files from a directory as a flat links page.
pub struct FindLinksServer {
    server: HttpServer,
    page_url: String,
}

impl FindLinksServer {
    /// Start a server that serves all files in the given directory.
    pub fn new(directory: &Path) -> Self {
        Self::new_at_path(directory, "/")
    }

    /// Start a server that serves the flat links page at the given URL path.
    pub fn new_at_path(directory: &Path, page_path: &str) -> Self {
        let mut files: HashMap<String, FileData> = HashMap::new();
        let mut filenames: Vec<String> = Vec::new();

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
            files.insert(filename.clone(), FileData::Bytes(bytes.into()));
            filenames.push(filename);
        }
        filenames.sort();

        let files = Arc::new(files);
        let filenames = Arc::new(filenames);
        let page_path = normalize_page_path(page_path);
        let request_page_path = page_path.clone();
        let server = HttpServer::start(move |request, server_uri| {
            handle_request(request, server_uri, &request_page_path, &files, &filenames)
        });
        let page_url = if page_path == "/" {
            server.url().to_string()
        } else {
            format!("{}{page_path}", server.url())
        };

        Self { server, page_url }
    }

    /// Start a server that serves the pinned registry artifacts used by tests.
    pub fn vendor() -> Self {
        let mut files: HashMap<String, FileData> = HashMap::new();
        let mut filenames: Vec<String> = Vec::new();

        for artifact in vendor_artifacts() {
            files.insert(artifact.filename.to_string(), FileData::Vendor(artifact));
            filenames.push(artifact.filename.to_string());
        }
        filenames.sort();

        let files = Arc::new(files);
        let filenames = Arc::new(filenames);
        let server = HttpServer::start(move |request, server_uri| {
            handle_request(request, server_uri, "/", &files, &filenames)
        });
        let page_url = server.url().to_string();

        Self { server, page_url }
    }

    /// The URL of the flat links page (for use with `--find-links`).
    pub fn url(&self) -> &str {
        &self.page_url
    }

    /// Return the URL of a file served by this index.
    pub fn file_url(&self, filename: &str) -> String {
        format!("{}/{filename}", self.server.url())
    }
}

fn handle_request(
    request: &Request,
    server_uri: &str,
    page_path: &str,
    files: &HashMap<String, FileData>,
    filenames: &[String],
) -> ResponseTemplate {
    let path = request.url.path();

    if if page_path == "/" {
        path == "/"
    } else {
        path.trim_end_matches('/') == page_path
    } {
        let links = filenames
            .iter()
            .map(|filename| format!("<a href=\"{server_uri}/{filename}\">{filename}</a>"))
            .collect::<Vec<_>>()
            .join("\n");
        let html = format!("<!DOCTYPE html>\n<html><body>\n{links}\n</body></html>");
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

fn normalize_page_path(page_path: &str) -> String {
    let page_path = page_path.trim_matches('/');
    if page_path.is_empty() {
        "/".to_string()
    } else {
        format!("/{page_path}")
    }
}

#[cfg(test)]
mod tests {
    use super::FindLinksServer;
    use crate::vendor::vendor_artifacts;

    #[test]
    fn vendor_server_construction_does_not_load_artifacts() {
        let _server = FindLinksServer::vendor();

        assert!(vendor_artifacts().all(|artifact| !artifact.is_loaded()));
    }
}
