use std::borrow::Cow;
use std::fmt::{Debug, Display};
use std::path::Path;

use same_file::is_same_file;
use tracing::{debug, trace};
use url::Url;

use uv_cache_info::CacheInfo;
use uv_cache_key::{CanonicalUrl, RepositoryUrl};
use uv_distribution_filename::ExpandedTags;
use uv_distribution_types::{
    BuildInfo, BuildVariables, ConfigSettings, ExtraBuildRequirement, ExtraBuildRequires,
    ExtraBuildVariables, InstalledDirectUrlDist, InstalledDist, InstalledDistKind,
    PackageConfigSettings, RequirementSource,
};
use uv_git_types::{GitLfs, GitOid, GitReference, GitUrl};
use uv_normalize::PackageName;
use uv_pep440::Version;
use uv_platform_tags::{AbiTag, IncompatibleTag, TagCompatibility, Tags};
use uv_pypi_types::{DirInfo, DirectUrl, VcsInfo, VcsKind};

use crate::InstallationStrategy;

/// Expected build settings when deciding whether to reuse an installed distribution.
#[derive(Debug, Clone, Copy)]
pub struct BuildSettings<'a> {
    pub config_settings: &'a ConfigSettings,
    pub config_settings_package: &'a PackageConfigSettings,
    pub extra_build_requires: &'a ExtraBuildRequires,
    pub extra_build_variables: &'a ExtraBuildVariables,
}

#[derive(Debug, Copy, Clone)]
pub(crate) enum RequirementSatisfaction {
    Mismatch,
    Satisfied,
    OutOfDate,
    CacheInvalid,
}

impl RequirementSatisfaction {
    /// Determine whether a requirement is satisfied by an installed distribution.
    pub(crate) fn check(
        name: &PackageName,
        distribution: &InstalledDist,
        source: &RequirementSource,
        version: Option<&Version>,
        installation: InstallationStrategy,
        tags: &Tags,
        build_settings: Option<BuildSettings<'_>>,
    ) -> Self {
        trace!(
            "Comparing installed with source: {:?} {:?}",
            distribution, source
        );

        // If the distribution was built with other settings, it is out of date.
        if let Some(build_settings) = build_settings
            && distribution.build_info().is_some_and(|dist_build_info| {
                let config_settings = config_settings_for(
                    name,
                    build_settings.config_settings,
                    build_settings.config_settings_package,
                );
                let extra_build_requires =
                    extra_build_requires_for(name, build_settings.extra_build_requires);
                let extra_build_variables =
                    extra_build_variables_for(name, build_settings.extra_build_variables);
                let build_info = BuildInfo::from_settings(
                    config_settings.into_owned(),
                    extra_build_requires.to_vec(),
                    extra_build_variables.cloned(),
                );
                dist_build_info != &build_info
            })
        {
            debug!("Build info mismatch for {name}: {distribution}");
            return Self::OutOfDate;
        }

        // Check that the installed distribution matches the requested source.
        let source_satisfaction = match source {
            // If the requirement comes from a registry, check by name.
            RequirementSource::Registry { specifier, .. } => {
                // If the installed distribution is _not_ from a registry, reject it if and only if
                // we're in a stateless install.
                //
                // For example: the `uv pip` CLI is stateful, in that it "respects"
                // already-installed packages in the virtual environment. So if you run `uv pip
                // install ./path/to/idna`, and then `uv pip install anyio` (which depends on
                // `idna`), we'll "accept" the already-installed `idna` even though it is implicitly
                // being "required" as a registry package.
                //
                // The `uv sync` CLI is stateless, in that all requirements must be defined
                // declaratively ahead-of-time. So if you `uv sync` to install `./path/to/idna` and
                // later `uv sync` to install `anyio`, we'll know (during that second sync) if the
                // already-installed `idna` should come from the registry or not.
                if installation == InstallationStrategy::Strict
                    && !matches!(distribution.kind, InstalledDistKind::Registry { .. })
                {
                    debug!("Distribution type mismatch for {name}: {distribution:?}");
                    return Self::Mismatch;
                }

                if !specifier.contains(distribution.version()) {
                    return Self::Mismatch;
                }
                Self::Satisfied
            }
            RequirementSource::Url {
                // We use the location since `direct_url.json` also stores this URL, e.g.
                // `pip install git+https://github.com/tqdm/tqdm@cc372d09dcd5a5eabdc6ed4cf365bdb0be004d44#subdirectory=.`
                // records `"url": "https://github.com/tqdm/tqdm"` in `direct_url.json`.
                location: requested_url,
                subdirectory: requested_subdirectory,
                ext: _,
                url: _,
            } => {
                let InstalledDistKind::Url(InstalledDirectUrlDist {
                    direct_url,
                    editable,
                    cache_info,
                    ..
                }) = &distribution.kind
                else {
                    return Self::Mismatch;
                };
                let DirectUrl::ArchiveUrl {
                    url: installed_url,
                    archive_info: _,
                    subdirectory: installed_subdirectory,
                } = direct_url.as_ref()
                else {
                    return Self::Mismatch;
                };

                if *editable {
                    return Self::Mismatch;
                }

                if requested_subdirectory != installed_subdirectory {
                    return Self::Mismatch;
                }

                if !CanonicalUrl::parse(installed_url).is_ok_and(|installed_url| {
                    installed_url == CanonicalUrl::new(requested_url.clone())
                }) {
                    return Self::Mismatch;
                }

                // If the requirement came from a local path, check freshness.
                if requested_url.scheme() == "file"
                    && let Ok(archive) = requested_url.to_file_path()
                {
                    Self::check_cache_info(distribution, &archive, cache_info.as_ref())
                } else {
                    Self::Satisfied
                }
            }
            RequirementSource::GitDirectory {
                url: _,
                git: requested_git,
                subdirectory: requested_subdirectory,
            } => {
                let InstalledDistKind::Url(InstalledDirectUrlDist { direct_url, .. }) =
                    &distribution.kind
                else {
                    return Self::Mismatch;
                };
                let DirectUrl::VcsUrl {
                    url: installed_url,
                    vcs_info:
                        installed_vcs @ VcsInfo {
                            vcs: VcsKind::Git, ..
                        },
                    subdirectory: installed_subdirectory,
                    path: None,
                } = direct_url.as_ref()
                else {
                    return Self::Mismatch;
                };

                if requested_subdirectory != installed_subdirectory {
                    debug!(
                        "Subdirectory mismatch: {:?} vs. {:?}",
                        installed_subdirectory, requested_subdirectory
                    );
                    return Self::Mismatch;
                }

                Self::check_git(requested_git, installed_url, installed_vcs)
            }
            RequirementSource::GitPath {
                url: _,
                git: requested_git,
                install_path: requested_path,
                ext: _,
            } => {
                let InstalledDistKind::Url(InstalledDirectUrlDist { direct_url, .. }) =
                    &distribution.kind
                else {
                    return Self::Mismatch;
                };
                let DirectUrl::VcsUrl {
                    url: installed_url,
                    vcs_info:
                        installed_vcs @ VcsInfo {
                            vcs: VcsKind::Git, ..
                        },
                    subdirectory: None,
                    path: Some(installed_path),
                } = direct_url.as_ref()
                else {
                    return Self::Mismatch;
                };

                if requested_path != installed_path {
                    debug!(
                        "Path mismatch: {:?} vs. {:?}",
                        installed_path, requested_path
                    );
                    return Self::Mismatch;
                }

                Self::check_git(requested_git, installed_url, installed_vcs)
            }
            RequirementSource::Path {
                install_path: requested_path,
                ext: _,
                url: _,
            } => {
                let InstalledDistKind::Url(InstalledDirectUrlDist {
                    direct_url,
                    cache_info,
                    ..
                }) = &distribution.kind
                else {
                    return Self::Mismatch;
                };
                let DirectUrl::ArchiveUrl {
                    url: installed_url,
                    archive_info: _,
                    subdirectory: None,
                } = direct_url.as_ref()
                else {
                    return Self::Mismatch;
                };

                Self::check_local_path(
                    distribution,
                    requested_path,
                    installed_url,
                    cache_info.as_ref(),
                )
            }
            RequirementSource::Directory {
                install_path: requested_path,
                editable: requested_editable,
                r#virtual: _,
                url: _,
            } => {
                let InstalledDistKind::Url(InstalledDirectUrlDist {
                    direct_url,
                    cache_info,
                    ..
                }) = &distribution.kind
                else {
                    return Self::Mismatch;
                };
                let DirectUrl::LocalDirectory {
                    url: installed_url,
                    dir_info:
                        DirInfo {
                            editable: installed_editable,
                        },
                    subdirectory: None,
                } = direct_url.as_ref()
                else {
                    return Self::Mismatch;
                };

                if requested_editable != installed_editable {
                    trace!(
                        "Editable mismatch: {:?} vs. {:?}",
                        *requested_editable,
                        installed_editable.unwrap_or_default()
                    );
                    return Self::Mismatch;
                }

                Self::check_local_path(
                    distribution,
                    requested_path,
                    installed_url,
                    cache_info.as_ref(),
                )
            }
        };
        if !matches!(source_satisfaction, Self::Satisfied) {
            return source_satisfaction;
        }

        // If the distribution isn't compatible with the current platform, it is a mismatch.
        if let Ok(Some(wheel_tags)) = distribution.read_tags() {
            if !wheel_tags.is_compatible(tags) {
                if let Some(hint) = generate_dist_compatibility_hint(wheel_tags, tags) {
                    debug!("Platform tags mismatch for `{distribution}`: {hint}");
                } else {
                    debug!("Platform tags mismatch for `{distribution}`");
                }
                return Self::Mismatch;
            }
        }

        // If a resolved version is provided, check that it matches the installed version.
        // This is needed for sources that don't include explicit version specifiers (e.g.,
        // directory dependencies with dynamic versioning), where the resolver may have determined
        // a new version should be installed.
        if let Some(version) = version {
            if distribution.version() != version {
                debug!(
                    "Installed version does not match resolved version for {name}: {} vs. {}",
                    distribution.version(),
                    version
                );
                return Self::OutOfDate;
            }
        }

        // Otherwise, assume the requirement is up-to-date.
        Self::Satisfied
    }

    /// Compare the Git details common to repository and in-repository path requirements.
    fn check_git(requested_git: &GitUrl, installed_url: &str, installed_vcs: &VcsInfo) -> Self {
        let requested_git_lfs = requested_git.lfs();
        let installed_git_lfs = installed_vcs.git_lfs.map(GitLfs::from).unwrap_or_default();
        if requested_git_lfs != installed_git_lfs {
            debug!(
                "Git LFS mismatch: {} (installed) vs. {} (requested)",
                installed_git_lfs, requested_git_lfs,
            );
            return Self::Mismatch;
        }

        if !RepositoryUrl::parse(installed_url)
            .is_ok_and(|installed_url| installed_url == *requested_git.repository())
        {
            debug!(
                "Repository mismatch: {:?} vs. {:?}",
                installed_url,
                requested_git.url()
            );
            return Self::Mismatch;
        }

        if !git_commit_matches(installed_vcs.commit_id.as_deref(), requested_git) {
            debug!(
                "Precise mismatch: {:?} vs. {:?}",
                installed_vcs.commit_id,
                requested_git.precise()
            );
            return Self::OutOfDate;
        }

        Self::Satisfied
    }

    /// Compare a local path requirement to an installed archive or directory.
    fn check_local_path(
        distribution: &InstalledDist,
        requested_path: &Path,
        installed_url: &str,
        cache_info: Option<&CacheInfo>,
    ) -> Self {
        let Some(installed_path) = Url::parse(installed_url)
            .ok()
            .and_then(|url| url.to_file_path().ok())
        else {
            return Self::Mismatch;
        };

        if !(requested_path == installed_path.as_path()
            || is_same_file(requested_path, &installed_path).unwrap_or(false))
        {
            trace!(
                "Path mismatch: {:?} vs. {:?}",
                requested_path, installed_path,
            );
            return Self::Mismatch;
        }

        Self::check_cache_info(distribution, requested_path, cache_info)
    }

    /// Compare the installed source's cache information to the current local path.
    fn check_cache_info(
        distribution: &InstalledDist,
        requested_path: &Path,
        cache_info: Option<&CacheInfo>,
    ) -> Self {
        let Some(cache_info) = cache_info else {
            return Self::OutOfDate;
        };
        match CacheInfo::from_path(requested_path) {
            Ok(current) if cache_info == &current => Self::Satisfied,
            Ok(_) => Self::OutOfDate,
            Err(err) => {
                debug!("Failed to read cached requirement for: {distribution} ({err})");
                Self::CacheInvalid
            }
        }
    }
}

/// Check an installed Git commit against a resolved commit or a full commit revision.
/// Branches, tags, and abbreviated revisions require resolution before they can be compared.
fn git_commit_matches(installed_commit: Option<&str>, requested_git: &GitUrl) -> bool {
    let requested_commit = requested_git
        .precise()
        .or_else(|| match requested_git.reference() {
            GitReference::BranchOrTagOrCommit(revision) => revision.parse::<GitOid>().ok(),
            GitReference::Branch(_)
            | GitReference::Tag(_)
            | GitReference::BranchOrTag(_)
            | GitReference::NamedRef(_)
            | GitReference::DefaultBranch => None,
        });

    if let (Some(installed_commit), Some(requested_commit)) = (installed_commit, requested_commit) {
        installed_commit.eq_ignore_ascii_case(requested_commit.as_str())
    } else {
        false
    }
}

/// Determine the [`ConfigSettings`] for the given package name.
fn config_settings_for<'settings>(
    name: &PackageName,
    config_settings: &'settings ConfigSettings,
    config_settings_package: &PackageConfigSettings,
) -> Cow<'settings, ConfigSettings> {
    if let Some(package_settings) = config_settings_package.get(name) {
        Cow::Owned(package_settings.clone().merge(config_settings.clone()))
    } else {
        Cow::Borrowed(config_settings)
    }
}

/// Determine the extra build requirements for the given package name.
fn extra_build_requires_for<'settings>(
    name: &PackageName,
    extra_build_requires: &'settings ExtraBuildRequires,
) -> &'settings [ExtraBuildRequirement] {
    extra_build_requires
        .get(name)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// Determine the extra build variables for the given package name.
fn extra_build_variables_for<'settings>(
    name: &PackageName,
    extra_build_variables: &'settings ExtraBuildVariables,
) -> Option<&'settings BuildVariables> {
    extra_build_variables.get(name)
}

/// Generate a hint for explaining tag compatibility issues.
// TODO(zanieb): We should refactor this to share logic with `generate_wheel_compatibility_hint`
fn generate_dist_compatibility_hint(wheel_tags: &ExpandedTags, tags: &Tags) -> Option<String> {
    let TagCompatibility::Incompatible(incompatible_tag) = wheel_tags.compatibility(tags) else {
        return None;
    };

    Some(match incompatible_tag {
        IncompatibleTag::Python => format_dist_compatibility_hint(
            wheel_tags.python_tags(),
            tags.python_tag().as_ref(),
            "using",
            |tag| tag.pretty(),
        ),
        IncompatibleTag::FreethreadedAbi => {
            let wheel_abi = wheel_tags
                .abi_tags()
                .map(|tag| match tag {
                    AbiTag::Abi3 => format!("the stable ABI (`{tag}`)"),
                    _ => {
                        if let Some(pretty) = tag.pretty() {
                            format!("the {pretty} ABI (`{tag}`)")
                        } else {
                            format!("`{tag}`")
                        }
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let current = tags.abi_tag().map_or_else(
                || "free-threaded Python".to_string(),
                |current| format_tag(&current, current.pretty()),
            );
            format!(
                "You're using {current}, but the distribution was built for {wheel_abi}, which requires a GIL-enabled interpreter"
            )
        }
        IncompatibleTag::Abi => format_dist_compatibility_hint(
            wheel_tags.abi_tags(),
            tags.abi_tag().as_ref(),
            "using",
            |tag| tag.pretty(),
        ),
        IncompatibleTag::Platform => format_dist_compatibility_hint(
            wheel_tags.platform_tags(),
            tags.platform_tag(),
            "on",
            |tag| tag.pretty(),
        ),
        IncompatibleTag::Variant | IncompatibleTag::Invalid | IncompatibleTag::AbiPythonVersion => {
            return None;
        }
    })
}

fn format_dist_compatibility_hint<'tag, T: Display + 'tag, P: Display>(
    wheel_tags: impl Iterator<Item = &'tag T>,
    current: Option<&T>,
    current_phrase: &str,
    pretty: impl Fn(&T) -> Option<P>,
) -> String {
    let current = current.map(|tag| format_tag(tag, pretty(tag)));
    let wheel_tags = wheel_tags
        .map(|tag| format_tag(tag, pretty(tag)))
        .collect::<Vec<_>>()
        .join(", ");
    if let Some(current) = current {
        format!(
            "The distribution is compatible with {wheel_tags}, but you're {current_phrase} {current}"
        )
    } else {
        format!("The distribution requires {wheel_tags}")
    }
}

fn format_tag(tag: &impl Display, pretty: Option<impl Display>) -> String {
    if let Some(pretty) = pretty {
        format!("{pretty} (`{tag}`)")
    } else {
        format!("`{tag}`")
    }
}
