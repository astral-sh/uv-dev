use std::collections::BTreeSet;

use tracing::debug;

use uv_normalize::PackageName;

/// The set of packages to install for a given category.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InstallSelection {
    /// Install all packages.
    #[default]
    All,
    /// Omit packages in the category.
    None,
    /// Install only packages in the category.
    Only,
}

impl InstallSelection {
    /// Determine the install selection from the command-line arguments.
    pub fn from_args(no_install: bool, only_install: bool) -> Self {
        if only_install {
            Self::Only
        } else if no_install {
            Self::None
        } else {
            Self::All
        }
    }
}

/// Minimal view of a package used to apply install filters.
#[derive(Debug, Clone, Copy)]
pub struct InstallTarget<'a> {
    /// The package name.
    pub name: &'a PackageName,
    /// Whether the package refers to a local source (path, directory, editable, etc.).
    pub is_local: bool,
}

#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    /// Select the project itself for installation.
    project: InstallSelection,
    /// Select workspace members (including the project itself) for installation.
    workspace: InstallSelection,
    /// Select local packages for installation.
    local: InstallSelection,
    /// Omit the specified packages from the resolution.
    no_install_package: Vec<PackageName>,
    /// Include only the specified packages in the resolution.
    only_install_package: Vec<PackageName>,
}

impl InstallOptions {
    pub fn new(
        project: InstallSelection,
        workspace: InstallSelection,
        local: InstallSelection,
        no_install_package: Vec<PackageName>,
        only_install_package: Vec<PackageName>,
    ) -> Self {
        Self {
            project,
            workspace,
            local,
            no_install_package,
            only_install_package,
        }
    }

    /// Returns `true` if a package passes the install filters.
    pub fn include_package(
        &self,
        target: InstallTarget<'_>,
        project_name: Option<&PackageName>,
        members: &BTreeSet<PackageName>,
    ) -> bool {
        let package_name = target.name;

        // If `--only-install-package` is set, only include specified packages.
        if !self.only_install_package.is_empty() {
            if self.only_install_package.contains(package_name) {
                return true;
            }
            debug!("Omitting `{package_name}` from resolution due to `--only-install-package`");
            return false;
        }

        // If `--only-install-local` is set, only include local packages.
        if self.local == InstallSelection::Only {
            if target.is_local {
                return true;
            }
            debug!("Omitting `{package_name}` from resolution due to `--only-install-local`");
            return false;
        }

        // If `--only-install-workspace` is set, only include the project and workspace members.
        if self.workspace == InstallSelection::Only {
            // Check if it's the project itself
            if let Some(project_name) = project_name
                && package_name == project_name
            {
                return true;
            }

            // Check if it's a workspace member
            if members.contains(package_name) {
                return true;
            }

            // Otherwise, exclude it
            debug!("Omitting `{package_name}` from resolution due to `--only-install-workspace`");
            return false;
        }

        // If `--only-install-project` is set, only include the project itself.
        if self.project == InstallSelection::Only {
            if let Some(project_name) = project_name
                && package_name == project_name
            {
                return true;
            }
            debug!("Omitting `{package_name}` from resolution due to `--only-install-project`");
            return false;
        }

        // If `--no-install-project` is set, remove the project itself.
        if self.project == InstallSelection::None
            && let Some(project_name) = project_name
            && package_name == project_name
        {
            debug!("Omitting `{package_name}` from resolution due to `--no-install-project`");
            return false;
        }

        // If `--no-install-workspace` is set, remove the project and any workspace members.
        if self.workspace == InstallSelection::None {
            // In some cases, the project root might be omitted from the list of workspace members
            // encoded in the lockfile. (But we already checked this above if `--no-install-project`
            // is set.)
            if self.project != InstallSelection::None
                && let Some(project_name) = project_name
                && package_name == project_name
            {
                debug!("Omitting `{package_name}` from resolution due to `--no-install-workspace`");
                return false;
            }

            if members.contains(package_name) {
                debug!("Omitting `{package_name}` from resolution due to `--no-install-workspace`");
                return false;
            }
        }

        // If `--no-install-local` is set, remove local packages.
        if self.local == InstallSelection::None {
            if target.is_local {
                debug!("Omitting `{package_name}` from resolution due to `--no-install-local`");
                return false;
            }
        }

        // If `--no-install-package` is provided, remove the requested packages.
        if self.no_install_package.contains(package_name) {
            debug!("Omitting `{package_name}` from resolution due to `--no-install-package`");
            return false;
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::str::FromStr;

    use anyhow::Error;
    use uv_normalize::PackageName;

    use super::{
        InstallOptions,
        InstallSelection::{self, All, None, Only},
        InstallTarget,
    };

    #[test]
    fn install_selection_from_args() {
        assert_eq!(InstallSelection::from_args(false, false), All);
        assert_eq!(InstallSelection::from_args(true, false), None);
        assert_eq!(InstallSelection::from_args(false, true), Only);
        assert_eq!(InstallSelection::from_args(true, true), Only);
    }

    #[test]
    fn include_package() -> Result<(), Error> {
        let project = PackageName::from_str("project")?;
        let member = PackageName::from_str("member")?;
        let local = PackageName::from_str("local")?;
        let remote = PackageName::from_str("remote")?;
        let members = BTreeSet::from([member.clone()]);

        for (project_selection, workspace_selection, local_selection, expected) in [
            (All, All, All, [true, true, true, true]),
            (None, All, All, [false, true, true, true]),
            (Only, All, All, [true, false, false, false]),
            (All, None, All, [false, false, true, true]),
            (All, Only, All, [true, true, false, false]),
            (All, All, None, [false, false, false, true]),
            (All, All, Only, [true, true, true, false]),
            (None, None, Only, [true, true, true, false]),
            (None, Only, None, [true, true, false, false]),
            (Only, None, None, [true, false, false, false]),
        ] {
            let options = InstallOptions::new(
                project_selection,
                workspace_selection,
                local_selection,
                vec![],
                vec![],
            );
            let include = |name, is_local| {
                options.include_package(InstallTarget { name, is_local }, Some(&project), &members)
            };
            assert_eq!(
                [
                    include(&project, true),
                    include(&member, true),
                    include(&local, true),
                    include(&remote, false),
                ],
                expected,
            );
        }

        Ok(())
    }
}
