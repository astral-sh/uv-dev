//! Read `pylock.toml` inputs and import their requirements for a new resolution.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use anyhow::Context;
use itertools::Itertools;
use tracing::info_span;

use uv_client::BaseClientBuilder;
use uv_configuration::{DependencyGroups, ExtrasSpecification, Upgrade};
use uv_distribution_types::UnresolvedRequirementSpecification;
use uv_lock::PylockToml;
use uv_normalize::{DefaultExtras, DefaultGroups};
use uv_pep508::RequirementOrigin;
use uv_requirements::PylockSource;

use crate::locked_requirements::LockedRequirements;

/// Read and parse a `pylock.toml` input, reusing the contents retained for stdin.
///
/// Returns the `install_path` (used to resolve relative package sources in the lock) alongside
/// the parsed [`PylockToml`]. For stdin and remote sources, the current working directory is used as
/// the install path.
pub async fn read_pylock_toml(
    pylock: &PylockSource,
    client_builder: &BaseClientBuilder<'_>,
) -> anyhow::Result<(PathBuf, PylockToml)> {
    let (install_path, content) = match pylock {
        PylockSource::Stdin(content) => (std::env::current_dir()?, Cow::Borrowed(content.as_str())),
        PylockSource::Remote(url) => {
            let client = client_builder.build()?;
            let response = client
                .for_host(url)
                .get(url::Url::from(url.clone()))
                .send()
                .await?;
            response.error_for_status_ref()?;
            let content = response.text().await?;
            (std::env::current_dir()?, Cow::Owned(content))
        }
        PylockSource::Local(path) => {
            let absolute = std::path::absolute(path)?;
            let install_path = absolute
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(PathBuf::new);
            let content = fs_err::tokio::read_to_string(path).await?;
            (install_path, Cow::Owned(content))
        }
    };

    let input = pylock.user_display();
    let lock = info_span!("toml::from_str pylock.toml", path = %input)
        .in_scope(|| toml::from_str::<PylockToml>(&content))
        .with_context(|| format!("Not a valid `pylock.toml` file: {input}"))?;

    Ok((install_path, lock))
}

/// Import requirements and preferences from a `pylock.toml`, without enforcing its artifact hashes.
pub async fn import_pylock_toml(
    source: &PylockSource,
    extras: &ExtrasSpecification,
    groups: &DependencyGroups,
    upgrade: &Upgrade,
    client_builder: &BaseClientBuilder<'_>,
) -> anyhow::Result<(Vec<UnresolvedRequirementSpecification>, LockedRequirements)> {
    let (install_path, lock) = read_pylock_toml(source, client_builder).await?;
    let extras = extras.with_defaults(DefaultExtras::default());
    let extras = extras
        .extra_names(lock.extras.iter())
        .cloned()
        .collect::<Vec<_>>();
    let groups = groups.with_defaults(DefaultGroups::from_groups(lock.default_groups.clone()));
    let groups = groups
        .group_names(lock.dependency_groups.iter())
        .chain(
            lock.default_groups
                .iter()
                .filter(|group| groups.contains_because_default(group)),
        )
        .unique()
        .cloned()
        .collect::<Vec<_>>();
    let origin = match source {
        PylockSource::Stdin(_) => Some(RequirementOrigin::File(PathBuf::from("-"))),
        PylockSource::Local(path) => Some(RequirementOrigin::File(path.clone())),
        PylockSource::Remote(_) => None,
    };
    let requirements = lock
        .to_requirements(&install_path, &extras, &groups)?
        .into_iter()
        .map(|mut requirement| {
            requirement.origin.clone_from(&origin);
            UnresolvedRequirementSpecification::from(requirement)
        })
        .collect();
    let preferences =
        LockedRequirements::from_pylock(&lock, &install_path, &extras, &groups, upgrade)?;
    Ok((requirements, preferences))
}
