use std::fmt::Write;
use std::str::FromStr;
use std::time::{Duration, SystemTimeError};

use anyhow::{Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::process::Command;
use url::Url;
use uv_bin_install::{Binary, find_matching_version_for_platform};
use uv_client::{
    BaseClient, BaseClientBuilder, RetriableError, WrappedReqwestError, fetch_with_url_fallback,
};
use uv_distribution_filename::{LegacySourceDistExtension, SourceDistExtension};
use uv_pep440::Version;
use uv_redacted::DisplaySafeUrl;
use uv_static::EnvVars;

use super::self_install::{InstallReceipt, LockedInstallation, ReleaseSource, executable_names};
use super::self_update::{is_update_needed, official_target_version_specifiers};
use crate::commands::ExitStatus;
use crate::printer::Printer;

struct Release {
    version: Version,
    urls: Vec<DisplaySafeUrl>,
    checksum_url: Option<DisplaySafeUrl>,
    sha256: Option<String>,
    format: SourceDistExtension,
    filename: String,
    api_origin: Option<Url>,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    url: DisplaySafeUrl,
}

fn github_api() -> Result<Url> {
    let github = std::env::var(EnvVars::UV_INSTALLER_GITHUB_BASE_URL).ok();
    let ghe = std::env::var(EnvVars::UV_INSTALLER_GHE_BASE_URL).ok();
    anyhow::ensure!(
        github.is_none() || ghe.is_none(),
        "Cannot set both UV_INSTALLER_GITHUB_BASE_URL and UV_INSTALLER_GHE_BASE_URL"
    );
    if let Some(ghe) = ghe {
        return Ok(Url::parse(&format!(
            "{}/api/v3/",
            ghe.trim_end_matches('/')
        ))?);
    }
    if let Some(github) = github {
        let mut url = Url::parse(&github)?;
        let domain = url.domain().context("GitHub base URL must have a domain")?;
        url.set_host(Some(&format!("api.{domain}")))?;
        url.set_path("/");
        url.set_query(None);
        url.set_fragment(None);
        return Ok(url);
    }
    Ok(Url::parse("https://api.github.com/")?)
}

async fn resolve_release(
    source: &ReleaseSource,
    version: Option<&str>,
    target: &str,
    client: &BaseClient,
    client_builder: &BaseClientBuilder<'_>,
    token: Option<&str>,
) -> Result<Release> {
    anyhow::ensure!(
        source.release_type == "github",
        "Unsupported release source `{}`",
        source.release_type
    );
    let format = if target.ends_with("-windows-msvc") {
        SourceDistExtension::Legacy(LegacySourceDistExtension::Zip)
    } else {
        SourceDistExtension::TarGz
    };
    let extension = if target.ends_with("-windows-msvc") {
        "zip"
    } else {
        "tar.gz"
    };
    let filename = format!("uv-{target}.{extension}");
    if source.owner == "astral-sh"
        && source.name == "uv"
        && std::env::var_os(EnvVars::UV_INSTALLER_GITHUB_BASE_URL).is_none()
        && std::env::var_os(EnvVars::UV_INSTALLER_GHE_BASE_URL).is_none()
    {
        let constraints = official_target_version_specifiers(version)?;
        let resolved = find_matching_version_for_platform(
            Binary::Uv,
            constraints.as_ref(),
            None,
            target,
            client,
            &client_builder.retry_policy(),
        )
        .await?;
        return Ok(Release {
            sha256: resolved.sha256().map(str::to_owned),
            urls: resolved.artifact_urls().to_vec(),
            format: resolved.archive_format(),
            version: resolved.version,
            checksum_url: None,
            filename,
            api_origin: None,
        });
    }

    let api = github_api()?;
    let mut url = api.clone();
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|()| anyhow::anyhow!("Invalid GitHub API URL"))?;
        segments
            .pop_if_empty()
            .extend(["repos", &source.owner, &source.name, "releases"]);
        if let Some(version) = version {
            segments.extend(["tags", version]);
        } else {
            segments.push("latest");
        }
    }
    let url = DisplaySafeUrl::from(url);
    let bytes = download_bytes(
        client,
        &url,
        token,
        Some(&api),
        "application/vnd.github+json",
    )
    .await?;
    let release: GithubRelease = serde_json::from_slice(&bytes)?;
    let version = Version::from_str(
        release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name),
    )?;
    let asset = |name: &str| -> Result<DisplaySafeUrl> {
        release
            .assets
            .iter()
            .find(|asset| asset.name == name)
            .map(|asset| asset.url.clone())
            .with_context(|| format!("Release `{}` has no `{name}` asset", release.tag_name))
    };
    Ok(Release {
        version,
        urls: vec![asset(&filename)?],
        checksum_url: Some(asset(&format!("{filename}.sha256"))?),
        sha256: None,
        format,
        filename,
        api_origin: Some(api),
    })
}

async fn download_bytes(
    client: &BaseClient,
    url: &DisplaySafeUrl,
    token: Option<&str>,
    api: Option<&Url>,
    accept: &str,
) -> Result<Vec<u8>, DownloadError> {
    let parsed = Url::from(url.clone());
    let mut request = client
        .for_host(url)
        .get(parsed.clone())
        .header("Accept", accept);
    if let Some(token) = token
        && (parsed.host_str() == Some("github.com")
            || api.is_some_and(|api| api.origin() == parsed.origin()))
    {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let response = request.send().await.map_err(|source| DownloadError::Http {
        url: url.clone(),
        source: source.into(),
    })?;
    let response = response
        .error_for_status()
        .map_err(|source| DownloadError::Http {
            url: url.clone(),
            source: source.into(),
        })?;
    Ok(response
        .bytes()
        .await
        .map_err(|source| DownloadError::Http {
            url: url.clone(),
            source: source.into(),
        })?
        .to_vec())
}

fn checksum_from_sidecar(bytes: &[u8], filename: &str) -> Result<String, DownloadError> {
    let text = std::str::from_utf8(bytes).map_err(|_| DownloadError::InvalidChecksum)?;
    let mut fields = text.split_whitespace();
    let digest = fields.next().ok_or(DownloadError::InvalidChecksum)?;
    if fields
        .next()
        .is_some_and(|name| name.trim_start_matches('*') != filename)
        || fields.next().is_some()
    {
        return Err(DownloadError::InvalidChecksum);
    }
    validate_checksum(digest)?;
    Ok(digest.to_owned())
}

fn validate_checksum(digest: &str) -> Result<(), DownloadError> {
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(DownloadError::InvalidChecksum);
    }
    Ok(())
}

async fn download_release(
    release: &Release,
    client: &BaseClient,
    builder: &BaseClientBuilder<'_>,
    token: Option<&str>,
) -> Result<tempfile::TempDir> {
    let bytes = fetch_with_url_fallback(
        &release.urls,
        builder.retry_policy(),
        "uv release archive",
        |url| async move {
            let checksum = if let Some(checksum) = &release.sha256 {
                validate_checksum(checksum)?;
                checksum.clone()
            } else {
                let checksum_url = match &release.checksum_url {
                    Some(checksum_url) => checksum_url.clone(),
                    None => DisplaySafeUrl::parse(&format!("{url}.sha256"))
                        .map_err(|_| DownloadError::InvalidChecksum)?,
                };
                let bytes = download_bytes(
                    client,
                    &checksum_url,
                    token,
                    release.api_origin.as_ref(),
                    "application/octet-stream",
                )
                .await?;
                checksum_from_sidecar(&bytes, &release.filename)?
            };
            let bytes = download_bytes(
                client,
                &url,
                token,
                release.api_origin.as_ref(),
                "application/octet-stream",
            )
            .await?;
            let actual = hex::encode(Sha256::digest(&bytes));
            if !actual.eq_ignore_ascii_case(&checksum) {
                return Err(DownloadError::ChecksumMismatch {
                    expected: checksum,
                    actual,
                });
            }
            Ok(bytes)
        },
    )
    .await?;
    let (directory, _) = uv_extract::stream::archive(
        std::io::Cursor::new(bytes),
        release.format,
        tempfile::tempdir()?,
    )
    .await?;
    Ok(directory)
}

pub(super) async fn self_update(
    version: Option<String>,
    token: Option<String>,
    dry_run: bool,
    printer: Printer,
    client_builder: BaseClientBuilder<'_>,
) -> Result<ExitStatus> {
    let executable = fs_err::canonicalize(std::env::current_exe()?)?;
    let installation = LockedInstallation::acquire(
        executable
            .parent()
            .context("Executable has no parent directory")?,
    )
    .await?;
    let (_, receipt) = InstallReceipt::for_executable(&executable)?;
    let current = Version::from_str(env!("CARGO_PKG_VERSION"))?;
    // Another process may have replaced this executable while we waited for the lock. Receipts
    // can be stale after a manual replacement, so compare the installed build itself.
    let installed = Command::new(&executable)
        .arg("--version")
        .env(EnvVars::UV_NO_CONFIG, "1")
        .output()
        .await
        .context("Failed to check the installed uv executable")?;
    let expected = format!("uv {}", uv_cli::version::uv_self_version());
    anyhow::ensure!(
        installed.status.success()
            && std::str::from_utf8(&installed.stdout)
                .is_ok_and(|output| output.trim_end() == expected),
        "The installed uv version changed; run `uv self update` again"
    );
    let client = client_builder.clone().retries(0).build()?;
    let target = uv_platform::build_target();
    writeln!(printer.stderr(), "Checking for updates...")?;
    let release = resolve_release(
        &receipt.source,
        version.as_deref(),
        &target,
        &client,
        &client_builder,
        token.as_deref(),
    )
    .await?;
    if !is_update_needed(&current, &release.version, version.is_some()) {
        writeln!(
            printer.stderr(),
            "You're already on version {current} of uv."
        )?;
        return Ok(ExitStatus::Success);
    }
    if dry_run {
        writeln!(
            printer.stderr_important(),
            "Would update uv from v{current} to v{}",
            release.version
        )?;
        return Ok(ExitStatus::Success);
    }
    let directory = download_release(&release, &client, &client_builder, token.as_deref()).await?;
    let source = if release.format == SourceDistExtension::TarGz {
        directory.path().join(format!("uv-{target}"))
    } else {
        directory.path().to_path_buf()
    };
    let mut updated = InstallReceipt::new(receipt.install_prefix.clone(), receipt.modify_path);
    updated.source = receipt.source.clone();
    updated.version = release.version.to_string();
    // Older uv releases may not contain all of today's companion executables.
    updated.binaries.clear();
    for name in executable_names() {
        if source.join(name).try_exists()? {
            updated.binaries.push((*name).to_owned());
        }
    }
    installation.install_binaries(&source, &updated.binaries, Some(&updated), Some(&receipt))?;
    writeln!(
        printer.stderr(),
        "Updated uv from {current} to {}",
        release.version
    )?;
    Ok(ExitStatus::Success)
}

#[derive(Debug, Error)]
enum DownloadError {
    #[error("Failed to download `{url}`")]
    Http {
        url: DisplaySafeUrl,
        #[source]
        source: WrappedReqwestError,
    },
    #[error("Invalid release archive checksum")]
    InvalidChecksum,
    #[error("Release archive checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("Download failed after {retries} retries in {duration:?}")]
    Retried {
        #[source]
        error: Box<Self>,
        retries: u32,
        duration: Duration,
    },
    #[error(transparent)]
    SystemTime(#[from] SystemTimeError),
}

impl RetriableError for DownloadError {
    fn retries(&self) -> u32 {
        if let Self::Retried { retries, .. } = self {
            *retries
        } else {
            0
        }
    }
    fn should_try_next_url(&self) -> bool {
        matches!(self, Self::Http { .. })
            || matches!(self, Self::Retried { error, .. } if error.should_try_next_url())
    }
    fn into_retried(self, retries: u32, duration: Duration) -> Self {
        Self::Retried {
            error: Box::new(self),
            retries,
            duration,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn checksum_mismatch_does_not_fallback_or_expose_github_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/archive.tar.gz"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"corrupted archive".to_vec()))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/fallback.tar.gz"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let release = Release {
            version: Version::new([9, 9, 9]),
            urls: vec![
                DisplaySafeUrl::parse(&format!("{}/archive.tar.gz", server.uri())).unwrap(),
                DisplaySafeUrl::parse(&format!("{}/fallback.tar.gz", server.uri())).unwrap(),
            ],
            checksum_url: None,
            sha256: Some("a".repeat(64)),
            format: SourceDistExtension::TarGz,
            filename: "archive.tar.gz".to_owned(),
            api_origin: None,
        };
        let builder = BaseClientBuilder::default().retries(0);
        let client = builder.build().unwrap();
        let error = download_release(&release, &client, &builder, Some("private-token"))
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<DownloadError>(),
            Some(DownloadError::ChecksumMismatch { .. })
        ));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(!requests[0].headers.contains_key("authorization"));
    }

    #[test]
    fn sidecar_requires_matching_filename() {
        let digest = "a".repeat(64);
        assert_eq!(
            checksum_from_sidecar(format!("{digest}  uv.tar.gz\n").as_bytes(), "uv.tar.gz")
                .unwrap(),
            digest
        );
        assert!(
            checksum_from_sidecar(format!("{digest}  other.tar.gz\n").as_bytes(), "uv.tar.gz")
                .is_err()
        );
        assert!(checksum_from_sidecar(b"1234", "uv.tar.gz").is_err());
    }
}
