//! Read a `pylock.toml` (PEP 751) file from an installation input.

use std::path::{Path, PathBuf};

use anyhow::Context;
use tracing::info_span;

use uv_client::BaseClientBuilder;
use uv_configuration::RequirementsInput;
use uv_lock::PylockToml;

/// Read a `pylock.toml` from a local path or remote URL and parse it.
///
/// Returns the `install_path` (used to resolve relative package sources in the lock) alongside
/// the parsed [`PylockToml`]. For remote sources, the current working directory is used as the
/// install path.
pub(crate) async fn read_pylock_toml(
    pylock: &RequirementsInput,
    client_builder: &BaseClientBuilder<'_>,
) -> anyhow::Result<(PathBuf, PylockToml)> {
    let (install_path, content) = match pylock {
        RequirementsInput::Stdin => (
            std::env::current_dir()?,
            uv_fs::read_stdin_to_string_transcode()?,
        ),
        RequirementsInput::Remote(url) => {
            let client = client_builder.build()?;
            let response = client.for_host(url).get(url.clone()).send().await?;
            response.error_for_status_ref()?;
            let content = response.text().await?;
            (std::env::current_dir()?, content)
        }
        RequirementsInput::Local(path) => {
            let absolute = std::path::absolute(path)?;
            let install_path = absolute
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(PathBuf::new);
            let content = fs_err::tokio::read_to_string(path).await?;
            (install_path, content)
        }
    };

    let pylock = pylock.user_display();
    let lock = info_span!("toml::from_str pylock.toml", path = %pylock)
        .in_scope(|| toml::from_str::<PylockToml>(&content))
        .with_context(|| format!("Not a valid `pylock.toml` file: {pylock}"))?;

    Ok((install_path, lock))
}
