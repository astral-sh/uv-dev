//! Serve a source archive containing a project in `packages/root`.

use anyhow::Result;
use indoc::indoc;
use sha2::{Digest, Sha256};
use wiremock::ResponseTemplate;

use crate::archive::write_tar_gz;
use crate::http_server::HttpServer;

const ARCHIVE_PATH: &str = "/subdirectory-test.tar.gz";
const PROJECT_PATH: &str = "subdirectory-test/packages/root";

/// A source archive whose nested `root` package has a dynamic version and depends on `anyio`.
pub struct SubdirectoryArchive {
    server: HttpServer,
    sha256: String,
}

impl SubdirectoryArchive {
    /// Generate the archive and serve it until the fixture is dropped.
    pub fn start() -> Result<Self> {
        let pyproject = indoc! {r#"
            [build-system]
            requires = ["hatchling"]
            build-backend = "hatchling.build"

            [project]
            name = "root"
            dynamic = ["version"]
            requires-python = ">=3.8"
            dependencies = ["anyio"]

            [tool.hatch.version]
            path = "root/__about__.py"
        "#};
        let mut archive = Vec::new();
        write_tar_gz(
            &mut archive,
            &[
                (&format!("{PROJECT_PATH}/pyproject.toml"), pyproject),
                (&format!("{PROJECT_PATH}/root/__init__.py"), ""),
                (
                    &format!("{PROJECT_PATH}/root/__about__.py"),
                    "__version__ = '0.0.1'\n",
                ),
            ],
        )?;
        let sha256 = hex::encode(Sha256::digest(&archive));
        let server = HttpServer::start(move |request, _| {
            if request.url.path() == ARCHIVE_PATH
                && (request.method == http::Method::GET || request.method == http::Method::HEAD)
            {
                ResponseTemplate::new(200).set_body_raw(archive.clone(), "application/gzip")
            } else {
                ResponseTemplate::new(404)
            }
        });
        Ok(Self { server, sha256 })
    }

    /// The URL of the archive, without a subdirectory fragment.
    pub fn url(&self) -> String {
        format!("{}{ARCHIVE_PATH}", self.server.url())
    }

    /// The SHA-256 digest of the served archive.
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}
