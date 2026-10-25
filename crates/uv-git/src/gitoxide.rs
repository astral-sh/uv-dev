use std::path::Path;
use std::sync::atomic::AtomicBool;

use anyhow::{Result, bail};
use gix::progress::Discard;
use gix::remote::Direction;
use uv_redacted::SafeUrl;
use uv_static::EnvVars;

use crate::git::GitError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Backend {
    Cli,
    Gitoxide,
}

impl Backend {
    pub(crate) fn for_remote(url: &SafeUrl, with_lfs: bool, disable_ssl: bool) -> Result<Self> {
        match std::env::var_os(EnvVars::UV_GIT_BACKEND).as_deref() {
            None => Ok(Self::Cli),
            Some(value) if value == "cli" => Ok(Self::Cli),
            Some(value) if value == "gitoxide" => {
                let inherited_repository = [
                    EnvVars::GIT_DIR,
                    EnvVars::GIT_WORK_TREE,
                    EnvVars::GIT_INDEX_FILE,
                    EnvVars::GIT_OBJECT_DIRECTORY,
                    EnvVars::GIT_ALTERNATE_OBJECT_DIRECTORIES,
                    EnvVars::GIT_COMMON_DIR,
                ]
                .iter()
                .any(|key| std::env::var_os(key).is_some());
                if matches!(url.scheme(), "http" | "https")
                    && !url.as_str().contains('?')
                    && url.fragment().is_none()
                    && !with_lfs
                    && !disable_ssl
                    && !inherited_repository
                {
                    Ok(Self::Gitoxide)
                } else {
                    Ok(Self::Cli)
                }
            }
            Some(_) => bail!("Invalid value for `UV_GIT_BACKEND`. Expected `cli` or `gitoxide`."),
        }
    }
}

pub(crate) fn fetch(
    path: &Path,
    url: &SafeUrl,
    refspecs: &[String],
    all_tags: bool,
    offline: bool,
) -> Result<()> {
    if offline {
        return Err(GitError::TransportNotAllowed.into());
    }
    let options = gix::open::Options::default()
        .config_overrides(["gitoxide.credentials.terminalPrompt=false"]);
    let mut repo = gix::open_opts(path, options)?;
    repo.committer_or_set_generic_fallback()?;
    let mut remote = repo
        .remote_at(url.as_str())?
        .with_refspecs(refspecs.iter().map(String::as_str), Direction::Fetch)?;
    if all_tags {
        remote = remote.with_fetch_tags(gix::remote::fetch::Tags::All);
    }
    remote
        .connect(Direction::Fetch)?
        .prepare_fetch(Discard, Default::default())?
        .receive(Discard, &AtomicBool::new(false))?;
    Ok(())
}
